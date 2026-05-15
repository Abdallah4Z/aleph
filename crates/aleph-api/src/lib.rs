pub mod routes;

use aleph_core::Config;
use anyhow::Result;

pub async fn run_api(config: &Config) -> Result<()> {
    let port = config.general.port;
    let data_dir = config.data_dir();
    routes::run_api(port, data_dir).await
}
