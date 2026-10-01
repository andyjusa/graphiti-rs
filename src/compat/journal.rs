use super::dto::{AddMemory, Prepared};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Mutex};

pub struct Journal {
    db: Mutex<Connection>,
    // A separate SQLite database holds an OS-released exclusive lock for this worker.
    _worker: Mutex<Connection>,
    capacity: usize,
}
pub struct Job {
    pub group: String,
    pub id: String,
    pub created: String,
    pub request: AddMemory,
    pub prepared: Option<Prepared>,
}
impl Job {
    pub fn validate_prepared(&self, value: &Prepared) -> Result<()> {
        value.validate()?;
        let e = &value.episode;
        ensure!(
            e.group_id == self.group
                && e.uuid == self.id
                && e.created_at == self.created
                && e.valid_at == self.created
                && e.name == self.request.name
                && e.content == self.request.episode_body
                && e.source == self.request.source
                && e.source_description == self.request.source_description,
            "prepared episode differs from its accepted identity or payload"
        );
        Ok(())
    }
}
pub struct Receipt {
    pub uuid: String,
    pub state: String,
}
impl Journal {
    pub fn open(dir: &Path, capacity: usize) -> Result<Self> {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        let m = std::fs::symlink_metadata(dir)?;
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "journal must be an ordinary private directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                m.permissions().mode() & 0o077 == 0,
                "journal directory must be owner-private"
            );
        }
        let worker = private_db(&dir.join("worker.sqlite"))?;
        worker.busy_timeout(std::time::Duration::ZERO)?;
        worker
            .execute_batch("BEGIN EXCLUSIVE")
            .context("journal already has an active worker")?;
        let db = private_db(&dir.join("jobs.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS jobs (group_id TEXT NOT NULL, uuid TEXT NOT NULL, digest TEXT NOT NULL, request TEXT NOT NULL, created_at TEXT NOT NULL, state TEXT NOT NULL, prepared TEXT, error TEXT, PRIMARY KEY(group_id,uuid));
            UPDATE jobs SET state='failed',error='uncertain model dispatch; automatic replay refused' WHERE state='running';")?;
        Ok(Self {
            db: Mutex::new(db),
            _worker: Mutex::new(worker),
            capacity,
        })
    }
    pub fn accept(&self, mut request: AddMemory) -> Result<Receipt> {
        let group = request.validate()?.to_owned();
        let id = request
            .uuid
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        request.uuid = Some(id.clone());
        request.group_id = Some(group.clone());
        let body = serde_json::to_string(&request)?;
        let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
        let mut db = self
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("journal unavailable"))?;
        let tx = db.transaction()?;
        let old: Option<(String, String, bool)> = tx
            .query_row(
                "SELECT digest,state,prepared IS NOT NULL FROM jobs WHERE group_id=? AND uuid=?",
                params![group, id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((old_digest, state, has_prepared)) = old {
            ensure!(
                old_digest == digest,
                "same-UUID update unsupported until patched Python baseline validation"
            );
            if state == "failed" && has_prepared {
                tx.execute(
                    "UPDATE jobs SET state='prepared',error=NULL WHERE group_id=? AND uuid=?",
                    params![group, id],
                )?;
                tx.commit()?;
                return Ok(Receipt {
                    uuid: id,
                    state: "prepared".into(),
                });
            }
            ensure!(
                state != "failed",
                "previous dispatch failed or is uncertain; automatic extraction retry refused"
            );
            return Ok(Receipt { uuid: id, state });
        }
        let active: usize = tx.query_row(
            "SELECT count(*) FROM jobs WHERE state IN ('queued','running','prepared')",
            [],
            |r| r.get(0),
        )?;
        ensure!(active < self.capacity, "ingestion queue full; retry later");
        let created = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        tx.execute("INSERT INTO jobs(group_id,uuid,digest,request,created_at,state) VALUES(?,?,?,?,?,'queued')",params![group,id,digest,body,created])?;
        tx.commit()?;
        Ok(Receipt {
            uuid: id,
            state: "queued".into(),
        })
    }
    pub fn next(&self) -> Result<Option<Job>> {
        let mut db = self
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("journal unavailable"))?;
        let tx = db.transaction()?;
        let job: Option<(String,String,String,String,Option<String>)>=tx.query_row("SELECT group_id,uuid,created_at,request,prepared FROM jobs WHERE state IN ('queued','prepared') ORDER BY created_at,uuid LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((group, id, created, request, prepared)) = job else {
            return Ok(None);
        };
        // Keep prepared durable until the graph commit has been reconciled.
        if prepared.is_none() {
            tx.execute(
                "UPDATE jobs SET state='running' WHERE group_id=? AND uuid=?",
                params![group, id],
            )?;
        }
        tx.commit()?;
        Ok(Some(Job {
            group,
            id,
            created,
            request: serde_json::from_str(&request)?,
            prepared: prepared.map(|s| serde_json::from_str(&s)).transpose()?,
        }))
    }
    pub fn prepared(&self, job: &Job, value: &Prepared) -> Result<()> {
        job.validate_prepared(value)?;
        self.db
            .lock()
            .map_err(|_| anyhow::anyhow!("journal unavailable"))?
            .execute(
                "UPDATE jobs SET state='prepared',prepared=? WHERE group_id=? AND uuid=?",
                params![serde_json::to_string(value)?, job.group, job.id],
            )?;
        Ok(())
    }
    pub fn finish(&self, job: &Job, success: bool) -> Result<()> {
        self.db
            .lock()
            .map_err(|_| anyhow::anyhow!("journal unavailable"))?
            .execute(
                "UPDATE jobs SET state=?,error=? WHERE group_id=? AND uuid=?",
                params![
                    if success { "committed" } else { "failed" },
                    if success {
                        None
                    } else {
                        Some("processing failed; no automatic model replay")
                    },
                    job.group,
                    job.id
                ],
            )?;
        Ok(())
    }
    pub fn pending(&self, group: &str) -> Result<bool> {
        Ok(self.db.lock().map_err(|_|anyhow::anyhow!("journal unavailable"))?.query_row("SELECT count(*)>0 FROM jobs WHERE group_id=? AND state IN ('queued','running','prepared')",[group],|r|r.get(0))?)
    }
}
fn private_db(path: &Path) -> Result<Connection> {
    if !path.exists() {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(path)?;
    }
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "invalid journal file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            m.permissions().mode() & 0o077 == 0,
            "journal file must be owner-private"
        );
    }
    Connection::open(path).context("cannot open private journal")
}
