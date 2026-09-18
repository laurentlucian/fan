//! Headless harness for fan-core: no window, no tray, just stats on stdout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use fan_core::{DeviceInfo, Engine, EngineConfig, EngineStats, OutputConfig};

const USAGE: &str = "\
fan-cli — headless harness for the fan-core engine

usage:
  fan-cli list
  fan-cli run --to <name-or-id> [--offset-ms <n>] [--to ...] [--source <name-or-id>]

  --to          output device, repeatable
  --offset-ms   delay for the --to it follows, -200..=200
  --source      capture source, defaults to the Windows default render endpoint

Devices match on exact id first, then case-insensitive substring of the name.
";

fn main() {
    env_logger::init();
    if let Err(e) = dispatch() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn dispatch() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("list") => list(),
        Some("run") => run(&args[1..]),
        None | Some("-h") | Some("--help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

fn list() -> Result<(), String> {
    for d in devices()? {
        let mark = if d.is_default { '*' } else { ' ' };
        println!("{mark} {:<40} {:<11?} {}", d.name, d.kind, d.id);
    }
    Ok(())
}

/// Flags as typed, before device names are resolved to endpoint ids.
#[derive(Default, Debug, PartialEq)]
struct Spec {
    outputs: Vec<(String, i32)>,
    source: Option<String>,
}

fn parse(args: &[String]) -> Result<Spec, String> {
    let mut spec = Spec::default();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("{flag} needs a value"))?;
        match flag {
            "--to" => spec.outputs.push((value.clone(), 0)),
            "--offset-ms" => {
                let ms: i32 = value
                    .parse()
                    .map_err(|_| format!("--offset-ms wants an integer, got {value:?}"))?;
                spec.outputs
                    .last_mut()
                    .ok_or_else(|| "--offset-ms must follow a --to".to_string())?
                    .1 = ms;
            }
            "--source" => spec.source = Some(value.clone()),
            other => return Err(format!("unknown flag {other:?}\n\n{USAGE}")),
        }
        i += 2;
    }
    if spec.outputs.is_empty() {
        return Err(format!("run needs at least one --to\n\n{USAGE}"));
    }
    Ok(spec)
}

fn run(args: &[String]) -> Result<(), String> {
    let spec = parse(args)?;
    let devices = devices()?;
    let mut config = EngineConfig::default();
    for (query, offset_ms) in &spec.outputs {
        let mut output = OutputConfig::new(resolve(&devices, query)?);
        output.offset_ms = *offset_ms;
        config.outputs.push(output);
    }
    if let Some(query) = &spec.source {
        config.source_device_id = Some(resolve(&devices, query)?);
    }

    let engine = Engine::start(config).map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler({
        let stop = Arc::clone(&stop);
        move || stop.store(true, Ordering::Relaxed)
    })
    .map_err(|e| e.to_string())?;

    while !stop.load(Ordering::Relaxed) {
        print_stats(&engine.stats(), &devices);
        std::thread::sleep(Duration::from_secs(1));
    }
    engine.stop();
    Ok(())
}

fn devices() -> Result<Vec<DeviceInfo>, String> {
    fan_core::list_outputs().map_err(|e| e.to_string())
}

fn resolve(devices: &[DeviceInfo], query: &str) -> Result<String, String> {
    if devices.iter().any(|d| d.id == query) {
        return Ok(query.to_string());
    }
    let needle = query.to_lowercase();
    let hits: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| d.name.to_lowercase().contains(&needle))
        .collect();
    match hits.as_slice() {
        [] => Err(format!("no device matches {query:?}")),
        [only] => Ok(only.id.clone()),
        many => {
            let names: Vec<&str> = many.iter().map(|d| d.name.as_str()).collect();
            Err(format!("{query:?} matches {}", names.join(", ")))
        }
    }
}

fn print_stats(stats: &EngineStats, devices: &[DeviceInfo]) {
    println!(
        "{} source {} peak {:.2}",
        if stats.running { "▶" } else { "■" },
        stats.source_name,
        stats.source_peak
    );
    for o in &stats.outputs {
        let name = devices
            .iter()
            .find(|d| d.id == o.device_id)
            .map(|d| d.name.as_str())
            .unwrap_or(&o.device_id);
        println!(
            "  {}{:<36} {:>6.1} ms  peak {:.2}  drift {:+.0} ppm  under {} over {}",
            if o.connected { ' ' } else { '!' },
            name,
            o.latency_ms,
            o.peak,
            o.drift_ppm,
            o.underruns,
            o.overruns
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fan_core::DeviceKind;

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|s| s.to_string()).collect()
    }

    fn device(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            name: name.into(),
            is_default: false,
            kind: DeviceKind::Other,
        }
    }

    #[test]
    fn offset_binds_to_preceding_to() {
        let spec = parse(&args(&[
            "--to",
            "a",
            "--to",
            "b",
            "--offset-ms",
            "-85",
            "--source",
            "s",
        ]))
        .unwrap();
        assert_eq!(
            spec,
            Spec {
                outputs: vec![("a".into(), 0), ("b".into(), -85)],
                source: Some("s".into())
            }
        );
    }

    #[test]
    fn rejects_bad_flags_and_empty_runs() {
        assert!(parse(&args(&["--to"])).is_err());
        assert!(parse(&args(&["--nope", "x"])).is_err());
        assert!(parse(&args(&["--to", "a", "--offset-ms", "x"])).is_err());
        assert!(parse(&args(&["--offset-ms", "10"])).is_err());
        assert!(parse(&args(&[])).is_err());
    }

    #[test]
    fn exact_id_beats_name_substring() {
        let devices = vec![
            device("{0.0.0}.speakers", "Speakers"),
            device("headset", "Headset (USB)"),
        ];
        assert_eq!(resolve(&devices, "headset").unwrap(), "headset");
        assert_eq!(resolve(&devices, "SPEAK").unwrap(), "{0.0.0}.speakers");
        assert!(resolve(&devices, "nope").is_err());
        assert!(resolve(
            &[device("1", "Speakers A"), device("2", "Speakers B")],
            "speakers"
        )
        .is_err());
    }
}
