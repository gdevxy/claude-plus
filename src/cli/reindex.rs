use anyhow::Result;

use super::format::plural;
use crate::ingest::{Ingestor, Rebuilt};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::store::Store;

pub fn run(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let mut store = super::open_store(paths)?;
    let rebuilt = rebuild(&mut store, prices)?;
    println!(
        "✻ Rebuilt {} from {}",
        plural(rebuilt.requests, "request", "requests"),
        plural(rebuilt.records, "stored record", "stored records")
    );
    Ok(())
}

pub fn rebuild(store: &mut Store, prices: &PriceTable) -> Result<Rebuilt> {
    let bar = super::progress_bar("Rebuilding")?;
    let rebuilt = Ingestor::new(store, prices).reindex(&mut |done, total| {
        bar.set_length(total as u64);
        bar.set_position(done as u64);
    })?;
    bar.finish_and_clear();
    Ok(rebuilt)
}
