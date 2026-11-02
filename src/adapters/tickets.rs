//! `TicketSystem` over the worktools clickup and monday clients, routed by
//! ref prefix with the same prefix lists `hv` uses for attribution.

use std::cell::OnceCell;

use harvest::Classifier;

use crate::ports::{PortError, PortResult, Source, Ticket, TicketRef, TicketSystem, TicketSystemKind};

pub struct Tickets {
    classifier: Classifier,
    clickup: OnceCell<Result<clickup::Client, String>>,
    monday: OnceCell<Result<(monday::Client, monday::Config), String>>,
}

fn err(kind: TicketSystemKind, e: impl std::fmt::Display) -> PortError {
    PortError {
        source: Source::TicketSystem(kind),
        message: e.to_string(),
    }
}

impl Tickets {
    /// Prefixes come from the Harvest config (HARVEST_MONDAY_PREFIXES and
    /// friends) when it loads, and the built-in defaults otherwise.
    pub fn new() -> Tickets {
        let classifier = harvest::Config::load().map(|c| c.classifier).unwrap_or_default();
        Tickets {
            classifier,
            clickup: OnceCell::new(),
            monday: OnceCell::new(),
        }
    }

    pub fn system_of(&self, reference: &TicketRef) -> TicketSystemKind {
        let prefix = reference.0.split('-').next().unwrap_or_default();
        if self
            .classifier
            .monday_prefixes
            .iter()
            .any(|p| p.eq_ignore_ascii_case(prefix))
        {
            TicketSystemKind::Monday
        } else {
            TicketSystemKind::ClickUp
        }
    }

    fn clickup(&self) -> PortResult<&clickup::Client> {
        self.clickup
            .get_or_init(|| {
                clickup::Config::load()
                    .and_then(|c| c.client(Vec::new()))
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(|e| err(TicketSystemKind::ClickUp, e))
    }

    fn monday(&self) -> PortResult<&(monday::Client, monday::Config)> {
        self.monday
            .get_or_init(|| {
                let config = monday::Config::load().map_err(|e| e.to_string())?;
                let client = config.client(Vec::new()).map_err(|e| e.to_string())?;
                Ok((client, config))
            })
            .as_ref()
            .map_err(|e| err(TicketSystemKind::Monday, e))
    }
}

impl Default for Tickets {
    fn default() -> Self {
        Tickets::new()
    }
}

impl TicketSystem for Tickets {
    fn fetch(&self, reference: &TicketRef) -> PortResult<Option<Ticket>> {
        let kind = self.system_of(reference);
        match kind {
            TicketSystemKind::ClickUp => match self.clickup()?.task(&reference.0) {
                Ok(task) => Ok(Some(Ticket {
                    reference: reference.clone(),
                    system: kind,
                    title: task.name,
                    status: task.status.status,
                })),
                Err(e) if e.api().is_some_and(|a| a.not_found()) => Ok(None),
                Err(e) => Err(err(kind, e)),
            },
            TicketSystemKind::Monday => {
                let (client, config) = self.monday()?;
                match client.item_by_ref(&config.ref_column_id, &reference.0) {
                    Ok(item) => {
                        let status = item
                            .column_value_by_id(&config.status_column_id)
                            .map(|cv| cv.text.clone())
                            .unwrap_or_default();
                        Ok(Some(Ticket {
                            reference: reference.clone(),
                            system: kind,
                            title: item.name,
                            status,
                        }))
                    }
                    // item_by_ref reports a missing item only in its message.
                    Err(e) if e.to_string().contains("no item with ref") => Ok(None),
                    Err(e) => Err(err(kind, e)),
                }
            }
        }
    }

    fn set_status(&self, reference: &TicketRef, status: &str) -> PortResult<()> {
        let kind = self.system_of(reference);
        match kind {
            TicketSystemKind::ClickUp => self
                .clickup()?
                .set_status(&reference.0, status)
                .map(|_| ())
                .map_err(|e| err(kind, e)),
            TicketSystemKind::Monday => {
                let (client, config) = self.monday()?;
                let item = client
                    .item_by_ref(&config.ref_column_id, &reference.0)
                    .map_err(|e| err(kind, e))?;
                client
                    .move_item(&config.status_column_id, &item.id, status)
                    .map(|_| ())
                    .map_err(|e| err(kind, e))
            }
        }
    }
}
