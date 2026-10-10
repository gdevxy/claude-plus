use std::path::PathBuf;

use anyhow::Result;

use super::format::plural;
use crate::ingest::{IngestReport, TranscriptFile, apply};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::transcript::parse_line;

pub fn run(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let mut store = super::open_store(paths)?;
    store.clear_derived()?;
    let mut report = IngestReport::default();
    let mut records = 0u64;
    for (file_id, path) in store.raw_files()? {
        let rows = store.raw_records(file_id, &[])?;
        let location = TranscriptFile::from_path(&PathBuf::from(&path));
        let mut writer = store.writer()?;
        for row in &rows {
            let Ok(parsed) = parse_line(&row.json) else {
                continue;
            };
            apply(
                &mut writer,
                prices,
                &location,
                row.file_id,
                row.line_no,
                &parsed,
                &mut report,
            )?;
            records += 1;
        }
        writer.commit()?;
    }
    println!(
        "✻ Rebuilt {} from {}",
        plural(report.requests_inserted, "request", "requests"),
        plural(records, "stored record", "stored records")
    );
    Ok(())
}
