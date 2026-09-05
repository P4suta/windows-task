//! Shared input invariants for libFuzzer and deterministic seed replay.

pub fn exercise(data: &[u8]) {
    if data.len() > 1024 * 1024 { return; }
    if let Ok(definition) = windows_task::xml::from_bytes(data) {
        if let Ok(xml) = windows_task::xml::to_string(&definition) {
            let decoded = windows_task::xml::from_bytes(xml.as_bytes()).expect("canonical XML must parse");
            assert_eq!(windows_task::xml::to_string(&decoded).expect("canonical output"), xml);
        }
    }
    for format in [windows_task::manifest::DocumentFormat::Json, windows_task::manifest::DocumentFormat::Toml, windows_task::manifest::DocumentFormat::Yaml] {
        if let Ok(manifest) = windows_task::manifest::TaskManifest::from_slice(data, format) {
            if let Ok(output) = manifest.to_string(format) {
                let decoded = windows_task::manifest::TaskManifest::from_slice(output.as_bytes(), format).expect("serialized manifest must parse");
                assert_eq!(decoded, manifest, "manifest serialization must preserve every value");
            }
        }
    }
    if let Ok(text) = std::str::from_utf8(data) {
        let _path = text.parse::<windows_task::TaskPath>();
        let _date = text.parse::<windows_task::model::TaskDateTime>();
        let _duration = text.parse::<windows_task::model::TaskDuration>();
        let _event = windows_task::history::from_event_xml(text);
        if let Ok(cron) = windows_task::schedule::CronSchedule::parse(text) {
            let boundary = "2026-01-01T00:00:00".parse().expect("fixed boundary");
            let _triggers = cron.compile(&boundary);
        }
    }
}
