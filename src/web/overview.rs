//! The start page: all groups and devices as a tree.

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::devices::{Device, flatten_groups};
use crate::error::Result;
use crate::status::{Status, device_status};
use crate::storage::{devices, groups, sensors, status};
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout, flash_message};

pub struct GroupInfo {
    pub id: i64,
    pub name: String,
}

/// One table row: either a group heading or a device.
pub struct OverviewRow {
    pub indent: usize,
    pub group: Option<GroupInfo>,
    pub device: Option<Device>,
    pub status_label: &'static str,
    pub status_class: &'static str,
    pub sensor_count: usize,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    layout: Layout,
    rows: Vec<OverviewRow>,
    flash: Option<&'static str>,
}

#[derive(Deserialize)]
pub struct FlashQuery {
    pub msg: Option<String>,
}

pub async fn home(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    let groups = groups::list_groups(&state.pool).await?;
    let devices = devices::list_devices(&state.pool).await?;

    let all_sensors = sensors::list_all_sensors(&state.pool).await?;
    let states = status::list_states(&state.pool).await?;
    let summary = |device: &Device| {
        let own: Vec<_> = all_sensors
            .iter()
            .filter(|s| s.device_id == device.id)
            .collect();
        let overall = device_status(
            own.iter()
                .map(|s| (s.enabled, states.get(&s.id).map(|st| st.status))),
        );
        (overall, own.len())
    };
    let device_row = |indent: usize, d: &Device| {
        let (overall, sensor_count) = summary(d);
        OverviewRow {
            indent,
            group: None,
            device: Some(d.clone()),
            status_label: overall.label(),
            status_class: overall.as_str(),
            sensor_count,
        }
    };
    let group_row = |indent: usize, id: i64, name: String| OverviewRow {
        indent,
        group: Some(GroupInfo { id, name }),
        device: None,
        status_label: Status::Unknown.label(),
        status_class: Status::Unknown.as_str(),
        sensor_count: 0,
    };

    let mut rows = Vec::new();
    let ungrouped: Vec<&Device> = devices.iter().filter(|d| d.group_id.is_none()).collect();
    if !ungrouped.is_empty() && !groups.is_empty() {
        rows.push(group_row(0, 0, "Ohne Gruppe".to_owned()));
    }
    let base = usize::from(!groups.is_empty());
    rows.extend(ungrouped.into_iter().map(|d| device_row(base, d)));
    for node in flatten_groups(&groups) {
        rows.push(group_row(
            node.depth - 1,
            node.group.id,
            node.group.name.clone(),
        ));
        rows.extend(
            devices
                .iter()
                .filter(|d| d.group_id == Some(node.group.id))
                .map(|d| device_row(node.depth, d)),
        );
    }

    let page = HomeTemplate {
        layout: user.layout(),
        rows,
        flash: flash_message(query.msg.as_deref()),
    };
    Ok(Html(page.render()?))
}
