use anyhow::Result;

use super::format::plural;
use crate::ingest::Ingestor;
use crate::paths::Paths;
use crate::pricing::PriceTable;

pub fn run(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let mut store = super::open_store(paths)?;
    let bar = super::progress_bar("Rebuilding")?;
    let rebuilt = Ingestor::new(&mut store, prices).reindex(&mut |done, total| {
        bar.set_length(total as u64);
        bar.set_position(done as u64);
    })?;
    bar.finish_and_clear();
    println!(
        "✻ Rebuilt {} from {}",
        plural(rebuilt.requests, "request", "requests"),
        plural(rebuilt.records, "stored record", "stored records")
    );
    Ok(())
}
