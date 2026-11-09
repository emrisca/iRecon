use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::AppState;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub created_at: String,
    pub path: String,
}

/// Adds a column to an existing database without a migration table. SQLite has
/// no `add column if not exists`, so the table is inspected first.
fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let mut statement = conn.prepare(&format!("pragma table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if !names.iter().any(|name| name == column) {
        conn.execute_batch(&format!(
            "alter table {table} add column {column} {definition}"
        ))?;
    }
    Ok(())
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(
        "create table if not exists project (
            id integer primary key autoincrement,
            name text not null,
            slug text not null unique,
            created_at text not null
        );

        create table if not exists frame (
            id integer primary key autoincrement,
            project_id integer not null references project(id) on delete cascade,
            path text not null,
            width integer not null,
            height integer not null,
            created_at text not null
        );

        create table if not exists class (
            id integer primary key autoincrement,
            project_id integer not null references project(id) on delete cascade,
            name text not null,
            color text not null,
            threshold real not null default 0.45,
            unique (project_id, name)
        );

        create table if not exists example (
            id integer primary key autoincrement,
            class_id integer not null references class(id) on delete cascade,
            path text not null,
            role text not null default 'positive',
            origin text not null default 'draw',
            bbox text,
            created_at text not null
        );

        create table if not exists source (
            id integer primary key autoincrement,
            project_id integer not null references project(id) on delete cascade,
            kind text not null check (kind in ('display','window','file','folder')),
            label text not null,
            target text not null,
            crop_x real, crop_y real, crop_w real, crop_h real,
            created_at text not null
        );

        create table if not exists candidate (
            id integer primary key autoincrement,
            project_id integer not null references project(id) on delete cascade,
            class_id integer not null references class(id) on delete cascade,
            path text not null,
            score real not null,
            state text not null default 'pending'
                check (state in ('pending','accepted','rejected','skipped')),
            created_at text not null
        );

        create table if not exists frame_truth (
            frame_id integer not null references frame(id) on delete cascade,
            class_id integer not null references class(id) on delete cascade,
            primary key (frame_id, class_id)
        );

        create index if not exists frame_project on frame(project_id);
        create index if not exists source_project on source(project_id);
        create index if not exists candidate_pending on candidate(project_id, state);
        create index if not exists class_project on class(project_id);
        create index if not exists example_class on example(class_id);",
    )?;

    // Added after the first schema: the source frame and the fingerprint that
    // makes an example matchable. Existing rows keep nulls until re-embedded.
    ensure_column(&conn, "example", "frame_id", "integer")?;
    ensure_column(&conn, "example", "embedding", "blob")?;
    ensure_column(&conn, "example", "embedding_model", "text")?;
    conn.execute_batch("create index if not exists example_frame on example(frame_id);")?;

    // Classes created before thresholds were measured still carry the old
    // 0.55 default, which accepted every unknown crop in the benchmark. Lift
    // that value, then move the 0.85 default to the 0.45 measured for the v3
    // fingerprint; v3 scores sit lower because the negative penalty is smaller.
    conn.execute_batch(
        "update class set threshold = 0.85 where threshold = 0.55;
         update class set threshold = 0.45 where threshold = 0.85;",
    )?;
    Ok(conn)
}

fn projects_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let root = dir.join("projects");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root)
}

fn row_to_project(row: &rusqlite::Row<'_>, root: &Path) -> rusqlite::Result<Project> {
    let slug: String = row.get(2)?;
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        path: root.join(&slug).to_string_lossy().to_string(),
        slug,
        created_at: row.get(3)?,
    })
}

pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "project".to_string()
    } else {
        out
    }
}

#[tauri::command]
pub fn projects_list(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Project>, String> {
    let root = projects_root(&app)?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("select id, name, slug, created_at from project order by created_at desc, id desc")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row_to_project(row, &root))
        .map_err(|e| e.to_string())?;
    let mut projects = Vec::new();
    for row in rows {
        projects.push(row.map_err(|e| e.to_string())?);
    }
    Ok(projects)
}

#[tauri::command]
pub fn project_create(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<Project, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the project a name".into());
    }
    if name.chars().count() > 64 {
        return Err("Keep the name under 64 characters".into());
    }
    let slug = slugify(&name);
    let root = projects_root(&app)?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "insert into project (name, slug, created_at) values (?1, ?2, datetime('now'))",
        params![name, slug],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            "A project with that name already exists".to_string()
        } else {
            e.to_string()
        }
    })?;
    let id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row("select created_at from project where id = ?1", params![id], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    drop(conn);

    let path = root.join(&slug);
    for sub in ["frames", "examples", "candidates", "sessions", "models"] {
        std::fs::create_dir_all(path.join(sub)).map_err(|e| e.to_string())?;
    }

    Ok(Project {
        id,
        name,
        slug,
        created_at,
        path: path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn project_delete(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let root = projects_root(&app)?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let slug: Option<String> = conn
        .query_row("select slug from project where id = ?1", params![id], |row| {
            row.get(0)
        })
        .ok();
    conn.execute("delete from project where id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    drop(conn);

    if let Some(slug) = slug {
        let path = root.join(slug);
        if path.exists() {
            std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
