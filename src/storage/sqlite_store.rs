use std::{path::Path, str::FromStr};

use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use crate::domain::error::DomainError;

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn connect(path: &Path) -> Result<Self, DomainError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                DomainError::Storage(format!("failed to create parent directory: {error}"))
            })?;
        }

        let db_url = format!("sqlite://{}", path.display());
        let options = SqliteConnectOptions::from_str(&db_url)
            .map_err(|error| DomainError::Storage(format!("invalid sqlite URL: {error}")))?
            .create_if_missing(true)
            .foreign_keys(true);

        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(|error| DomainError::Storage(format!("failed to connect sqlite: {error}")))?;

        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    async fn migrate(&self) -> Result<(), DomainError> {
        super::migrations::migrate(self.pool()).await
    }
}
