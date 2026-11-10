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
    /// Counted in the same query as the project, so the Projects page never
    /// shows a stale or guessed number.
    pub examples: i64,
    pub frames: i64,
    pub classes: i64,
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

        create table if not exists setting (
            key text primary key,
            value text not null
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

        create table if not exists capture_session (
            id integer primary key autoincrement,
            project_id integer not null references project(id) on delete cascade,
            started_at text not null,
            ended_at text,
            width integer not null default 0,
            height integer not null default 0,
            frames integer not null default 0,
            detections integer not null default 0,
            model text,
            note text
        );

        create table if not exists session_frame (
            id integer primary key autoincrement,
            session_id integer not null references capture_session(id) on delete cascade,
            frame_index integer not null,
            path text not null,
            width integer not null,
            height integer not null,
            at_ms integer not null,
            boxes text not null,
            proposals text not null,
            unique (session_id, frame_index)
        );

        create index if not exists frame_project on frame(project_id);
        create table if not exists project_detector (
            id integer primary key autoincrement,
            project_id integer not null references project(id) on delete cascade,
            name text not null,
            path text not null,
            input_size integer not null,
            active integer not null default 0,
            created_at text not null
        );

        create index if not exists session_project on capture_session(project_id);
        create index if not exists project_detector_project on project_detector(project_id);
        create table if not exists project_classifier (
            id integer primary key autoincrement,
            project_id integer not null references project(id) on delete cascade,
            name text not null,
            path text not null,
            input_size integer not null,
            labels text not null,
            active integer not null default 0,
            created_at text not null
        );
        create index if not exists project_classifier_project on project_classifier(project_id);
        create index if not exists session_frame_session on session_frame(session_id);
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
    ensure_column(&conn, "project_detector", "active", "integer not null default 0")?;
    // Minimum match score for boxes and review candidates. Zero keeps the old
    // behaviour; the Capture lab slider raises it.
    ensure_column(&conn, "project", "match_floor", "real not null default 0.0")?;
    conn.execute_batch(
        "create index if not exists example_frame on example(frame_id);
         create index if not exists example_pool on example(class_id, role, id);",
    )?;

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
        examples: row.get(4)?,
        frames: row.get(5)?,
        classes: row.get(6)?,
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

fn project_rows(conn: &Connection, root: &Path) -> Result<Vec<Project>, String> {
    let mut stmt = conn
        .prepare(
            "select p.id, p.name, p.slug, p.created_at,
                    (select count(*) from example e
                     join class c on c.id = e.class_id
                     where c.project_id = p.id),
                    (select count(*) from frame f where f.project_id = p.id),
                    (select count(*) from class c where c.project_id = p.id)
             from project p
             order by p.created_at desc, p.id desc",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row_to_project(row, root))
        .map_err(|e| e.to_string())?;
    let mut projects = Vec::new();
    for row in rows {
        projects.push(row.map_err(|e| e.to_string())?);
    }
    Ok(projects)
}

#[tauri::command]
pub fn projects_list(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Project>, String> {
    let root = projects_root(&app)?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    project_rows(&conn, &root)
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
        examples: 0,
        frames: 0,
        classes: 0,
    })
}

#[tauri::command]
pub fn project_rename(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
    name: String,
) -> Result<Project, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the project a name".into());
    }
    if name.chars().count() > 64 {
        return Err("Keep the name under 64 characters".into());
    }
    let root = projects_root(&app)?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let clash: Option<i64> = conn
        .query_row(
            "select id from project where lower(name) = lower(?1) and id != ?2",
            params![name, id],
            |row| row.get(0),
        )
        .ok();
    if clash.is_some() {
        return Err("A project with that name already exists".into());
    }
    // Only the display name changes. The slug names the project folder, and
    // frame and example paths are stored absolute, so moving it would break them.
    let changed = conn
        .execute(
            "update project set name = ?2 where id = ?1",
            params![id, name],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("That project is gone".to_string());
    }
    conn.query_row(
        "select p.id, p.name, p.slug, p.created_at,
                (select count(*) from example e
                 join class c on c.id = e.class_id
                 where c.project_id = p.id),
                (select count(*) from frame f where f.project_id = p.id),
                (select count(*) from class c where c.project_id = p.id)
         from project p where p.id = ?1",
        params![id],
        |row| row_to_project(row, &root),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn project_match_floor(
    state: tauri::State<'_, AppState>,
    project_id: i64,
) -> Result<f32, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "select match_floor from project where id = ?1",
        params![project_id],
        |row| row.get(0),
    )
    .map_err(|_| "That project is gone".to_string())
}

#[tauri::command]
pub fn project_set_match_floor(
    state: tauri::State<'_, AppState>,
    project_id: i64,
    floor: f32,
) -> Result<f32, String> {
    let floor = floor.clamp(0.0, 1.0);
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "update project set match_floor = ?2 where id = ?1",
        params![project_id, floor],
    )
    .map_err(|e| e.to_string())?;
    Ok(floor)
}

#[tauri::command]
pub fn setting_get(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<Option<String>, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("A setting needs a name".into());
    }
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    Ok(conn
        .query_row(
            "select value from setting where key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok())
}

#[tauri::command]
pub fn setting_set(
    state: tauri::State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("A setting needs a name".into());
    }
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "insert into setting (key, value) values (?1, ?2)
         on conflict(key) do update set value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("irecon-db-{tag}-{stamp}.sqlite"))
    }

    #[test]
    fn project_rows_count_frames_classes_and_examples() {
        let path = temp_db("counts");
        let conn = open(&path).expect("database opens");
        conn.execute_batch(
            "insert into project (name, slug, created_at) values ('Traffic', 'traffic', '2026-01-01');
             insert into project (name, slug, created_at) values ('Hud', 'hud', '2026-01-02');
             insert into class (project_id, name, color) values (1, 'car', '#fff');
             insert into class (project_id, name, color) values (1, 'sign', '#000');
             insert into example (class_id, path, created_at) values (1, 'a.png', '2026-01-03');
             insert into example (class_id, path, created_at) values (1, 'b.png', '2026-01-03');
             insert into example (class_id, path, created_at) values (2, 'c.png', '2026-01-03');
             insert into frame (project_id, path, width, height, created_at) values (1, 'f1.png', 16, 16, '2026-01-03');
             insert into frame (project_id, path, width, height, created_at) values (2, 'f3.png', 16, 16, '2026-01-03');",
        )
        .expect("fixture inserts");

        let rows = project_rows(&conn, Path::new("/root")).expect("projects list");
        let counts: Vec<(String, i64, i64, i64)> = rows
            .into_iter()
            .map(|p| (p.name, p.examples, p.classes, p.frames))
            .collect();
        assert_eq!(
            counts,
            vec![("Hud".to_string(), 0, 0, 1), ("Traffic".to_string(), 3, 2, 1)]
        );

        let floor: f32 = conn
            .query_row("select match_floor from project where id = 1", [], |row| row.get(0))
            .expect("match floor column");
        assert_eq!(floor, 0.0);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}
