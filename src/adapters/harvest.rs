//! `TimeTracker` over the worktools harvest client.

use std::cell::OnceCell;

use harvest::{Client, Date, EntryQuery, StartInput, System, TimeEntry};

use crate::ports::{PortError, PortResult, Source, Ticket, TicketRef, TimeTracker, Timer, TimerId};

pub struct Harvest {
    project: String,
    task: String,
    client: OnceCell<Result<Client, String>>,
}

fn err(e: impl std::fmt::Display) -> PortError {
    PortError {
        source: Source::Harvest,
        message: e.to_string(),
    }
}

impl Harvest {
    pub fn new(project: &str, task: &str) -> Harvest {
        Harvest {
            project: project.to_string(),
            task: task.to_string(),
            client: OnceCell::new(),
        }
    }

    /// The client, built on first use so commands that never touch Harvest
    /// do not need its credentials.
    fn client(&self) -> PortResult<&Client> {
        self.client
            .get_or_init(|| {
                harvest::Config::load()
                    .and_then(|c| c.client(Vec::new()))
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(err)
    }

    fn timer(&self, client: &Client, entry: &TimeEntry) -> Timer {
        let attribution = client.classifier().classify(entry);
        let ticket = match attribution.system {
            System::ClickUp | System::Monday if !attribution.ref_.is_empty() => Some(TicketRef(attribution.ref_)),
            _ => None,
        };
        Timer {
            id: TimerId(entry.id),
            ticket,
            notes: entry.notes.clone(),
        }
    }
}

impl TimeTracker for Harvest {
    fn running(&self) -> PortResult<Option<Timer>> {
        let client = self.client()?;
        Ok(client.running_entry().map_err(err)?.map(|e| self.timer(client, &e)))
    }

    fn entry_today(&self, ticket: &TicketRef) -> PortResult<Option<TimerId>> {
        let client = self.client()?;
        let today = Date::today();
        let query = EntryQuery {
            from: today,
            to: today,
            ..EntryQuery::default()
        };
        let entries = client.entries(&query).map_err(err)?;
        Ok(entries
            .iter()
            .filter(|e| self.timer(client, e).ticket.as_ref() == Some(ticket))
            .max_by_key(|e| e.updated_at)
            .map(|e| TimerId(e.id)))
    }

    fn start(&self, ticket: &Ticket) -> PortResult<TimerId> {
        let client = self.client()?;
        let target = client.resolve_target(&self.project, &self.task).map_err(err)?;
        let input = StartInput {
            target,
            notes: format!("{} - {}", ticket.reference.0, ticket.title),
            ..StartInput::default()
        };
        client.start_timer(&input).map(|e| TimerId(e.id)).map_err(err)
    }

    fn stop(&self, id: TimerId) -> PortResult<()> {
        self.client()?.stop_timer(id.0).map(|_| ()).map_err(err)
    }

    fn restart(&self, id: TimerId) -> PortResult<()> {
        self.client()?.restart_timer(id.0).map(|_| ()).map_err(err)
    }

    fn delete(&self, id: TimerId) -> PortResult<()> {
        self.client()?.delete_entry(id.0).map_err(err)
    }
}
