use std::collections::HashMap;

use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use chrono::{TimeDelta, Utc};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::model::{DormantRoute, ProjectRegistration, SelectionTarget, SessionRoute};

const RESTART_ROUTE_GRACE_SECONDS: i64 = 120;

pub(super) struct DeviceConnection {
    pub connection_id: Uuid,
    pub commands: mpsc::Sender<GatewayCommand>,
    pub disconnected: CancellationToken,
}

pub(super) struct PendingRequest {
    pub device_id: String,
    pub connection_id: Uuid,
    pub response: oneshot::Sender<BridgeResult>,
}

#[derive(Default)]
pub(super) struct BrokerState {
    pub devices: HashMap<String, DeviceConnection>,
    pub projects: HashMap<String, (String, ProjectRegistration)>,
    pub sessions: HashMap<String, SessionRoute>,
    pub generations: HashMap<(String, String), u64>,
    pub pending: HashMap<String, PendingRequest>,
    pub dormant: HashMap<(String, String), DormantRoute>,
}

impl BrokerState {
    pub fn disconnect(&mut self, device_id: &str, connection_id: Uuid) {
        let is_current = self
            .devices
            .get(device_id)
            .is_some_and(|value| value.connection_id == connection_id);
        if !is_current {
            return;
        }
        if let Some(connection) = self.devices.remove(device_id) {
            connection.disconnected.cancel();
        }
        let now = Utc::now();
        let owned_projects = self
            .projects
            .iter()
            .filter(|(_, (owner, _))| owner == device_id)
            .map(|(scope, (_, project))| (scope.clone(), project.clone()))
            .collect::<Vec<_>>();
        let routes = self
            .sessions
            .values()
            .filter(|route| route.device_id == device_id)
            .cloned()
            .collect::<Vec<_>>();
        for route in routes {
            if matches!(route.target, SelectionTarget::Project)
                && route.expires_at > now
                && let Some((scope, project)) = owned_projects
                    .iter()
                    .find(|(_, project)| project.thread_id == route.thread_id)
            {
                self.dormant.insert(
                    (device_id.to_owned(), route.thread_id.clone()),
                    DormantRoute {
                        route: route.clone(),
                        project_scope: scope.clone(),
                        binding_id: project.binding_id.clone(),
                        resume_until: route
                            .expires_at
                            .min(now + TimeDelta::seconds(RESTART_ROUTE_GRACE_SECONDS)),
                    },
                );
            }
        }
        self.projects.retain(|_, (owner, _)| owner != device_id);
        self.sessions
            .retain(|_, route| route.device_id != device_id);
        self.pending.retain(|_, pending| {
            pending.device_id != device_id || pending.connection_id != connection_id
        });
    }
}
