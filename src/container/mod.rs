use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

use crate::{PgrollerError, Result};

/// Manages a PostgreSQL Testcontainer for migration testing.
pub struct PgContainer {
    container: ContainerAsync<Postgres>,
    connection_string: String,
}

impl PgContainer {
    /// Start a new PostgreSQL container with the specified version.
    pub async fn start(postgres_version: &str) -> Result<Self> {
        let image = Postgres::default().with_tag(postgres_version);

        let container = image.start().await.map_err(|e| {
            PgrollerError::Container(format!("Failed to start Postgres container: {}", e))
        })?;

        let host_port = container.get_host_port_ipv4(5432).await.map_err(|e| {
            PgrollerError::Container(format!("Failed to get container port: {}", e))
        })?;

        let host = container.get_host().await.map_err(|e| {
            PgrollerError::Container(format!("Failed to get container host: {}", e))
        })?;

        let connection_string = format!(
            "host={} port={} user=postgres password=postgres dbname=postgres",
            host, host_port
        );

        Ok(Self {
            container,
            connection_string,
        })
    }

    /// Get the connection string for this container.
    pub fn connection_string(&self) -> String {
        self.connection_string.clone()
    }

    /// Install PostgreSQL extensions in the container database.
    pub async fn install_extensions(&self, extensions: &[String]) -> Result<()> {
        if extensions.is_empty() {
            return Ok(());
        }

        let (client, connection) =
            tokio_postgres::connect(&self.connection_string, tokio_postgres::NoTls)
                .await
                .map_err(|e| {
                    PgrollerError::Container(format!("Failed to connect to container: {}", e))
                })?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("Container connection error: {}", e);
            }
        });

        for ext in extensions {
            let sql = format!("CREATE EXTENSION IF NOT EXISTS \"{}\"", ext);
            client.execute(&sql, &[]).await.map_err(|e| {
                PgrollerError::Container(format!("Failed to install extension '{}': {}", ext, e))
            })?;
        }

        Ok(())
    }

    /// Get a reference to the underlying container (for lifetime management).
    pub fn inner(&self) -> &ContainerAsync<Postgres> {
        &self.container
    }
}
