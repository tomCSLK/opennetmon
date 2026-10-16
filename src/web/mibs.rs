//! MIB files page (admin only): upload vendor MIBs, see what was loaded, search objects.
//! Also the JSON search behind the name suggestions of the custom OID field.

use std::collections::HashSet;

use askama::Template;
use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::mib::{self, MibObject, Origin};
use crate::storage::mibs::{self as store, NewMibFile};
use crate::web::auth::{CsrfForm, CurrentUser};
use crate::web::{AppState, Layout, flash_message, iso_time_html};

/// Upper bounds for one upload; MIB files are small text files.
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
const MAX_FILES: usize = 300;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILE_NAME_CHARS: usize = 120;
const SEARCH_LIMIT: usize = 50;
const SUGGEST_LIMIT: usize = 20;
const DESCRIPTION_CHARS: usize = 240;

#[derive(Deserialize)]
pub struct MibsQuery {
    q: Option<String>,
    msg: Option<String>,
}

struct ModuleView {
    name: String,
    objects: usize,
    ok: bool,
    problem: String,
}

struct FileView {
    id: i64,
    file_name: String,
    size_kb: String,
    uploaded_at: String,
    modules: Vec<ModuleView>,
}

struct ObjectView {
    name: String,
    module: String,
    oid: String,
    kind: &'static str,
    type_name: String,
    units: String,
    access: String,
    description: String,
    pollable: bool,
    suggestion: String,
}

struct UploadResult {
    accepted: usize,
    modules: usize,
    replaced: u64,
    skipped: Vec<String>,
}

#[derive(Template)]
#[template(path = "mibs.html")]
struct PageTemplate {
    layout: Layout,
    files: Vec<FileView>,
    bundled: Vec<ModuleView>,
    total_objects: usize,
    total_modules: usize,
    query: String,
    results: Vec<ObjectView>,
    flash: Option<&'static str>,
    upload: Option<UploadResult>,
    error: Option<String>,
}

fn shorten(text: &str, max: usize) -> String {
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > max {
        text.chars().take(max - 1).collect::<String>() + "…"
    } else {
        text
    }
}

/// What to put into the OID field for an object: scalars with `.0`, columns with a trailing dot.
fn suggestion(o: &MibObject) -> String {
    match o.kind {
        mib::ObjectKind::Scalar => format!("{}.0", o.name),
        mib::ObjectKind::Column => format!("{}.", o.name),
        _ => o.name.clone(),
    }
}

fn object_view(o: &MibObject) -> ObjectView {
    ObjectView {
        name: o.name.clone(),
        module: o.module.clone(),
        oid: mib::dotted(&o.oid),
        kind: o.kind.label(),
        type_name: match (&o.type_name, o.base) {
            (Some(t), Some(b)) if !mib::is_builtin_type(t) => format!("{t} ({})", b.label()),
            (Some(t), _) => t.clone(),
            (None, Some(b)) => b.label().to_owned(),
            (None, None) => String::new(),
        },
        units: o.units.clone().unwrap_or_default(),
        access: o.access.clone().unwrap_or_default(),
        description: shorten(o.description.as_deref().unwrap_or(""), DESCRIPTION_CHARS),
        pollable: o.is_pollable(),
        suggestion: suggestion(o),
    }
}

fn module_view(r: &mib::ModuleReport) -> ModuleView {
    let mut problems = Vec::new();
    if r.shadowed {
        problems.push("übersprungen: wird mit netmon mitgeliefert".to_owned());
    }
    if !r.missing_modules.is_empty() {
        problems.push(format!("fehlt: {}", r.missing_modules.join(", ")));
    }
    if !r.unresolved.is_empty() {
        problems.push(format!(
            "{} {} nicht auflösbar",
            r.unresolved.len(),
            if r.unresolved.len() == 1 {
                "Definition"
            } else {
                "Definitionen"
            }
        ));
    }
    ModuleView {
        name: r.name.clone(),
        objects: r.objects,
        ok: r.is_ok(),
        problem: problems.join(" · "),
    }
}

async fn render(
    state: &AppState,
    user: &CurrentUser,
    query: &str,
    flash: Option<&'static str>,
    upload: Option<UploadResult>,
    error: Option<String>,
) -> Result<String> {
    let index = state.mib_index();
    let reports = index.reports();
    let files = store::list_files(&state.pool)
        .await?
        .into_iter()
        .map(|f| FileView {
            modules: reports
                .iter()
                .filter(|r| r.origin == Origin::Imported(f.id))
                .map(module_view)
                .collect(),
            id: f.id,
            size_kb: format!("{:.1} KB", f.size as f64 / 1024.0).replace('.', ","),
            uploaded_at: iso_time_html(&f.uploaded_at),
            file_name: f.file_name,
        })
        .collect();
    let bundled = reports
        .iter()
        .filter(|r| r.origin == Origin::Bundled)
        .map(module_view)
        .collect();
    let results = index
        .search(query, SEARCH_LIMIT)
        .into_iter()
        .map(object_view)
        .collect();
    let page = PageTemplate {
        layout: user.layout(),
        files,
        bundled,
        total_objects: index.len(),
        total_modules: reports.iter().filter(|r| !r.shadowed).count(),
        query: query.to_owned(),
        results,
        flash,
        upload,
        error,
    };
    Ok(page.render()?)
}

pub async fn show(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<MibsQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let q = query.q.unwrap_or_default();
    Ok(Html(
        render(
            &state,
            &user,
            q.trim(),
            flash_message(query.msg.as_deref()),
            None,
            None,
        )
        .await?,
    ))
}

/// Keeps only the last path component and printable characters of an uploaded file name.
fn clean_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_FILE_NAME_CHARS)
        .collect();
    if cleaned.trim().is_empty() {
        "unbenannt.mib".to_owned()
    } else {
        cleaned
    }
}

pub async fn upload(
    State(state): State<AppState>,
    user: CurrentUser,
    mut multipart: Multipart,
) -> Result<Response> {
    user.require_admin()?;
    let mut csrf = None;
    let mut uploads: Vec<(String, Vec<u8>)> = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| Error::Invalid(format!("Der Upload ist unvollständig oder zu groß: {e}")))?
    {
        match field.name() {
            Some("csrf_token") => csrf = Some(field.text().await.unwrap_or_default()),
            Some("files") => {
                let name = clean_file_name(field.file_name().unwrap_or_default());
                let data = field.bytes().await.map_err(|e| {
                    Error::Invalid(format!("Die Datei {name} konnte nicht gelesen werden: {e}"))
                })?;
                if data.is_empty() && name == "unbenannt.mib" {
                    continue; // the browser sends an empty part when no file was chosen
                }
                if uploads.len() >= MAX_FILES {
                    return Err(Error::Invalid(format!(
                        "Höchstens {MAX_FILES} Dateien auf einmal."
                    )));
                }
                uploads.push((name, data.to_vec()));
            }
            _ => {}
        }
    }
    user.verify_csrf(csrf.as_deref().unwrap_or_default())?;

    let bundled: HashSet<String> = mib::bundled_index()
        .reports()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for (name, data) in uploads {
        if data.len() > MAX_FILE_BYTES {
            skipped.push(format!("{name}: größer als 2 MB"));
            continue;
        }
        let text = String::from_utf8_lossy(&data).replace('\0', "");
        let modules: Vec<String> = mib::parser::parse(&text)
            .into_iter()
            .map(|m| m.name)
            .collect();
        if modules.is_empty() {
            skipped.push(format!(
                "{name}: keine MIB-Definition („… DEFINITIONS ::= BEGIN“) gefunden"
            ));
            continue;
        }
        if modules.iter().all(|m| bundled.contains(m)) {
            skipped.push(format!(
                "{name}: {} wird mit netmon mitgeliefert",
                modules.join(", ")
            ));
            continue;
        }
        files.push(NewMibFile {
            file_name: name,
            modules,
            content: text,
        });
    }
    if files.is_empty() && skipped.is_empty() {
        let html = render(
            &state,
            &user,
            "",
            None,
            None,
            Some("Bitte mindestens eine Datei auswählen.".to_owned()),
        )
        .await?;
        return Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response());
    }
    let replaced = store::save_files(&state.pool, &files).await?;
    state.reload_mibs().await?;
    let result = UploadResult {
        accepted: files.len(),
        modules: files.iter().map(|f| f.modules.len()).sum(),
        replaced,
        skipped,
    };
    tracing::info!(
        user = %user.username,
        accepted = result.accepted,
        skipped = result.skipped.len(),
        replaced = result.replaced,
        "MIB files uploaded"
    );
    Ok(Html(render(&state, &user, "", None, Some(result), None).await?).into_response())
}

pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    axum::Form(form): axum::Form<CsrfForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    if !store::delete_file(&state.pool, id).await? {
        return Err(Error::NotFound);
    }
    state.reload_mibs().await?;
    tracing::info!(user = %user.username, file = id, "MIB file deleted");
    Ok(Redirect::to("/mibs?msg=mib_deleted").into_response())
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
}

#[derive(Serialize)]
pub struct Suggestion {
    name: String,
    module: String,
    oid: String,
    kind: &'static str,
    #[serde(rename = "type")]
    type_name: String,
    units: String,
    description: String,
    pollable: bool,
    suggestion: String,
    /// Index columns of a table column (what has to be appended), empty otherwise.
    index: String,
}

/// Name suggestions for the custom OID field (admin only, like creating sensors).
pub async fn search_json(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<Suggestion>>> {
    user.require_admin()?;
    let index = state.mib_index();
    let q = query.q.unwrap_or_default();
    // Only the name part matters: "ifInOctets.3" searches "ifInOctets"; "IF-MIB::x" searches "x".
    let name = q
        .rsplit("::")
        .next()
        .unwrap_or_default()
        .split('.')
        .next()
        .unwrap_or_default();
    Ok(Json(
        index
            .search(name, SUGGEST_LIMIT)
            .into_iter()
            .map(|o| {
                let v = object_view(o);
                let index = if o.kind == mib::ObjectKind::Column {
                    o.index.join(", ")
                } else {
                    String::new()
                };
                Suggestion {
                    index,
                    name: v.name,
                    module: v.module,
                    oid: v.oid,
                    kind: v.kind,
                    type_name: v.type_name,
                    units: v.units,
                    description: shorten(&v.description, 160),
                    pollable: v.pollable,
                    suggestion: v.suggestion,
                }
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use crate::web::testutil::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};

    const VENDOR_MIB: &str = r#"VENDOR-MIB DEFINITIONS ::= BEGIN
IMPORTS MODULE-IDENTITY, OBJECT-TYPE, enterprises FROM SNMPv2-SMI;
vendorMIB MODULE-IDENTITY LAST-UPDATED "202401010000Z" ORGANIZATION "V" CONTACT-INFO "x" DESCRIPTION "x"
    ::= { enterprises 64000 }
vendorTemperature OBJECT-TYPE SYNTAX INTEGER (-40..120) UNITS "Grad Celsius" MAX-ACCESS read-only STATUS current
    DESCRIPTION "Board temperature." ::= { vendorMIB 1 }
vendorFan OBJECT-TYPE SYNTAX INTEGER { ok(1), failed(2) } MAX-ACCESS read-only STATUS current
    DESCRIPTION "Fan state." ::= { vendorMIB 2 }
vendorPackets OBJECT-TYPE SYNTAX Counter32 MAX-ACCESS read-only STATUS current
    DESCRIPTION "Packets." ::= { vendorMIB 3 }
END
"#;

    /// A multipart upload request with the CSRF token and the given files.
    fn upload_request(cookie: &str, csrf: &str, files: &[(&str, &str)]) -> Request<Body> {
        let boundary = "netmonTestBoundary7MA4YWxk";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{csrf}\r\n"
        );
        for (name, content) in files {
            body.push_str(&format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n{content}\r\n"
            ));
        }
        body.push_str(&format!("--{boundary}--\r\n"));
        Request::builder()
            .method("POST")
            .uri("/mibs/upload")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header(header::COOKIE, cookie)
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn only_admins_may_use_the_mib_pages() {
        let h = harness(true).await;
        assert_eq!(
            send(&h.app, get("/mibs", None)).await.status,
            StatusCode::SEE_OTHER
        );
        let (viewer, vcsrf) = login(&h, "bert").await;
        assert_eq!(
            send(&h.app, get("/mibs", Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&h.app, get("/mibs/search.json?q=sys", Some(&viewer)))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        let reply = send(
            &h.app,
            upload_request(&viewer, &vcsrf, &[("v.mib", VENDOR_MIB)]),
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        let (admin, _) = login(&h, "anna").await;
        let reply = send(
            &h.app,
            upload_request(&admin, "wrong", &[("v.mib", VENDOR_MIB)]),
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert!(
            crate::storage::mibs::list_files(&h.state.pool)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_page_shows_bundled_mibs_and_searches() {
        let h = harness(true).await;
        let (admin, _) = login(&h, "anna").await;
        let page = send(&h.app, get("/mibs?q=hrProcessorLoad", Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("IF-MIB") && page.body.contains("HOST-RESOURCES-MIB"));
        assert!(
            page.body.contains("1.3.6.1.2.1.25.3.3.1.2"),
            "search result with OID"
        );
        assert!(page.body.contains("hrProcessorLoad."), "column suggestion");
        let json = send(
            &h.app,
            get("/mibs/search.json?q=IF-MIB::ifInOctets.3", Some(&admin)),
        )
        .await;
        assert!(
            json.body.contains("\"name\":\"ifInOctets\"")
                && json.body.contains("\"suggestion\":\"ifInOctets.\""),
            "{}",
            json.body
        );
    }

    #[tokio::test]
    async fn uploads_are_parsed_stored_indexed_replaced_and_deleted() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let reply = send(
            &h.app,
            upload_request(
                &admin,
                &csrf,
                &[
                    ("vendor.mib", VENDOR_MIB),
                    ("notes.txt", "just text"),
                    ("RFC1213-MIB.mib", "RFC1213-MIB DEFINITIONS ::= BEGIN END"),
                ],
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert!(
            reply.body.contains("1 Datei mit 1 Modul übernommen"),
            "{}",
            reply.body
        );
        assert!(reply.body.contains("notes.txt: keine MIB-Definition"));
        assert!(
            reply
                .body
                .contains("RFC1213-MIB.mib: RFC1213-MIB wird mit netmon mitgeliefert")
        );
        assert!(reply.body.contains("vendor.mib") && reply.body.contains("VENDOR-MIB"));
        let (oid, _) = h
            .state
            .mib_index()
            .resolve_input("vendorTemperature.0")
            .unwrap();
        assert_eq!(crate::mib::dotted(&oid), "1.3.6.1.4.1.64000.1.0");

        // Uploading the module again replaces the first file.
        let newer = VENDOR_MIB.replace("Board temperature.", "Board temperature (v2).");
        let reply = send(
            &h.app,
            upload_request(&admin, &csrf, &[("vendor-v2.mib", &newer)]),
        )
        .await;
        assert!(
            reply.body.contains("1 ältere Fassung(en) ersetzt"),
            "{}",
            reply.body
        );
        let files = crate::storage::mibs::list_files(&h.state.pool)
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            h.state.mib_index().by_name("vendorTemperature", None)[0]
                .description
                .as_deref(),
            Some("Board temperature (v2).")
        );

        let reply = send(
            &h.app,
            post(
                &format!("/mibs/files/{}/delete", files[0].id),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(reply.location(), "/mibs?msg=mib_deleted");
        assert!(
            h.state
                .mib_index()
                .resolve_input("vendorTemperature.0")
                .is_err(),
            "index rebuilt without the file"
        );
        let missing = send(
            &h.app,
            post(
                &format!("/mibs/files/{}/delete", files[0].id),
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_file_with_a_missing_dependency_is_listed_with_the_problem() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let child = "CHILD-MIB DEFINITIONS ::= BEGIN IMPORTS OBJECT-TYPE FROM SNMPv2-SMI parentNode FROM PARENT-MIB;
            childValue OBJECT-TYPE SYNTAX INTEGER MAX-ACCESS read-only STATUS current DESCRIPTION \"x\" ::= { parentNode 1 } END";
        let reply = send(
            &h.app,
            upload_request(&admin, &csrf, &[("child.mib", child)]),
        )
        .await;
        assert!(reply.body.contains("fehlt: PARENT-MIB"), "{}", reply.body);
        assert!(reply.body.contains("1 Definition nicht auflösbar"));
    }

    #[tokio::test]
    async fn sensors_can_be_created_by_mib_name_with_mode_unit_and_name_from_the_mib() {
        use crate::devices::{DeviceInput, Polling};
        use crate::sensors::snmp::OidMode;
        use crate::sensors::spec::SensorSpec;
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        send(
            &h.app,
            upload_request(&admin, &csrf, &[("vendor.mib", VENDOR_MIB)]),
        )
        .await;
        let profile = add_v2c_profile(&h, "P", "public").await;
        let device = crate::storage::devices::save_device(
            &h.state.pool,
            None,
            DeviceInput {
                name: "Switch".to_owned(),
                host: "192.0.2.1".to_owned(),
                group_id: None,
                profile_id: profile,
                polling: Polling::default(),
            },
        )
        .await
        .unwrap();
        let path = format!("/devices/{device}/sensors");
        for oid in [
            "vendorTemperature",
            "VENDOR-MIB::vendorPackets.0",
            "vendorFan.0",
        ] {
            let body = format!(
                "csrf_token={csrf}&kind=oid&name=&oid={}&mode=auto&scale=1&unit=",
                oid.replace("::", "%3A%3A")
            );
            let reply = send(&h.app, post(&path, Some(&admin), &body)).await;
            assert_eq!(reply.status, StatusCode::SEE_OTHER, "{oid}: {}", reply.body);
        }
        let sensors = crate::storage::sensors::list_sensors(&h.state.pool, device)
            .await
            .unwrap();
        let find = |name: &str| {
            sensors
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("{name} missing: {sensors:?}"))
        };
        assert_eq!(
            find("vendorTemperature.0").spec,
            SensorSpec::Oid {
                oid: vec![1, 3, 6, 1, 4, 1, 64000, 1, 0],
                mode: OidMode::Gauge,
                scale: 1.0,
                unit: Some("Grad Celsius".to_owned())
            }
        );
        assert!(
            matches!(
                &find("vendorPackets.0").spec,
                SensorSpec::Oid {
                    mode: OidMode::Counter,
                    ..
                }
            ),
            "counters become rates"
        );
        assert!(matches!(
            &find("vendorFan.0").spec,
            SensorSpec::Oid {
                mode: OidMode::Gauge,
                unit: None,
                ..
            }
        ));

        // The list shows MIB names instead of numbers.
        let page = send(&h.app, get(&path, Some(&admin))).await;
        assert!(
            page.body
                .contains("vendorTemperature.0 (VENDOR-MIB) · Wert, Einheit Grad Celsius"),
            "{}",
            page.body
        );

        // Explicit mode and unit override the MIB; unknown names are explained.
        let body = format!(
            "csrf_token={csrf}&kind=oid&name=Temp+roh&oid=vendorTemperature.0&mode=counter&scale=1&unit=K"
        );
        assert_eq!(
            send(&h.app, post(&path, Some(&admin), &body)).await.status,
            StatusCode::SEE_OTHER
        );
        let raw = crate::storage::sensors::list_sensors(&h.state.pool, device)
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.name == "Temp roh")
            .unwrap();
        assert!(
            matches!(raw.spec, SensorSpec::Oid { mode: OidMode::Counter, unit: Some(ref u), .. } if u == "K")
        );
        let body =
            format!("csrf_token={csrf}&kind=oid&name=&oid=gibtEsNicht.0&mode=auto&scale=1&unit=");
        let reply = send(&h.app, post(&path, Some(&admin), &body)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(reply.body.contains("in keiner geladenen MIB bekannt"));
    }
}
