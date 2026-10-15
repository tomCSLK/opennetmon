//! The start page: all groups and devices as a tree.

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::devices::{Device, flatten_groups};
use crate::error::Result;
use crate::storage::{devices, groups};
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
    msg: Option<String>,
}

pub async fn home(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    let groups = groups::list_groups(&state.pool).await?;
    let devices = devices::list_devices(&state.pool).await?;

    let mut rows = Vec::new();
    let ungrouped: Vec<&Device> = devices.iter().filter(|d| d.group_id.is_none()).collect();
    if !ungrouped.is_empty() && !groups.is_empty() {
        rows.push(OverviewRow {
            indent: 0,
            group: Some(GroupInfo {
                id: 0,
                name: "Ohne Gruppe".to_owned(),
            }),
            device: None,
        });
    }
    let base = usize::from(!groups.is_empty());
    rows.extend(ungrouped.into_iter().map(|d| OverviewRow {
        indent: base,
        group: None,
        device: Some(d.clone()),
    }));
    for node in flatten_groups(&groups) {
        rows.push(OverviewRow {
            indent: node.depth - 1,
            group: Some(GroupInfo {
                id: node.group.id,
                name: node.group.name.clone(),
            }),
            device: None,
        });
        rows.extend(
            devices
                .iter()
                .filter(|d| d.group_id == Some(node.group.id))
                .map(|d| OverviewRow {
                    indent: node.depth,
                    group: None,
                    device: Some(d.clone()),
                }),
        );
    }

    let page = HomeTemplate {
        layout: user.layout(),
        rows,
        flash: flash_message(query.msg.as_deref()),
    };
    Ok(Html(page.render()?))
}
