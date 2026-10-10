use std::path::PathBuf;

use anyhow::Result;

use super::format::plural;
use crate::ingest::{IngestReport, TranscriptFile, apply};
use crate::paths::Paths;
use crate::pricing::PriceTable;
use crate::transcript::parse_line;

pub fn run(paths: &Paths, prices: &PriceTable) -> Result<()> {
    let mut store = super::open_store(paths)?;
    let mut writer = store.writer()?;
    writer.clear_derived()?;
    let mut report = IngestReport::default();
    let mut records = 0u64;
    let files = writer.raw_files()?;
    let bar = super::progress_bar("Rebuilding")?;
    bar.set_length(files.len() as u64);
    for (file_id, path) in files {
        let location = TranscriptFile::from_path(&PathBuf::from(&path));
        for row in writer.raw_records(file_id, &[])? {
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
        bar.inc(1);
    }
    writer.commit()?;
    bar.finish_and_clear();
    println!(
        "✻ Rebuilt {} from {}",
        plural(report.requests_inserted, "request", "requests"),
        plural(records, "stored record", "stored records")
    );
    Ok(())
}
