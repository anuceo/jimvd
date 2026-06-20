use anyhow::Result;
use plotters::prelude::*;
use std::fs;

/// A point in an evolution timeline for plotting.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvolutionPoint {
    pub operations:         usize,
    pub factor_utilization: f64,
}

pub fn write_csv(path: &str, rows: &[(&str, usize, &metrics::Metrics)]) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record(&[
        "runner", "scale", "p50_us", "p95_us", "p99_us",
        "throughput", "factor_util", "uaf", "storage_bytes",
    ])?;
    for (runner, scale, m) in rows {
        wtr.write_record(&[
            runner.to_string(),
            scale.to_string(),
            m.p50_latency_us.to_string(),
            m.p95_latency_us.to_string(),
            m.p99_latency_us.to_string(),
            m.throughput_ops_sec.to_string(),
            m.factor_utilization.to_string(),
            m.uaf.to_string(),
            m.storage_bytes.to_string(),
        ])?;
    }
    wtr.flush()?;
    Ok(())
}

pub fn write_json(path: &str, value: &impl serde::Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    fs::write(path, json)?;
    Ok(())
}

pub fn plot_uaf_vs_scale(
    path: &str,
    series: &[(&str, Vec<(usize, f64)>)],
) -> Result<()> {
    let root = BitMapBackend::new(path, (1024, 768)).into_drawing_area();
    root.fill(&WHITE)?;

    let max_x = series.iter()
        .flat_map(|(_, pts)| pts.iter().map(|(x, _)| *x))
        .max()
        .unwrap_or(1_000_000);
    let max_y = series.iter()
        .flat_map(|(_, pts)| pts.iter().map(|(_, y)| *y))
        .fold(1.0f64, f64::max);

    let mut chart = ChartBuilder::on(&root)
        .caption("UAF vs Scale", ("sans-serif", 30))
        .margin(20)
        .x_label_area_size(40)
        .y_label_area_size(60)
        .build_cartesian_2d(0usize..max_x, 0.0f64..max_y * 1.1)?;

    chart.configure_mesh().draw()?;

    let colors: [&RGBColor; 5] = [&RED, &BLUE, &GREEN, &MAGENTA, &CYAN];
    for (i, (label, points)) in series.iter().enumerate() {
        let color = colors[i % colors.len()];
        chart.draw_series(LineSeries::new(points.iter().cloned(), color))?
            .label(*label)
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], color));
    }

    chart.configure_series_labels().border_style(&BLACK).draw()?;
    root.present()?;
    Ok(())
}

/// A single row of per-phase metric data, engine-agnostic.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PhaseMetricRow {
    pub runner:             String,
    pub phase:              String,
    pub phase_index:        usize,
    pub ops_into_phase:     usize,
    pub total_ops:          usize,
    pub p50_us:             u64,
    pub p99_us:             u64,
    pub factor_utilization: f64,
    pub uaf:                f64,
    pub memory_bytes:       u64,
    pub storage_bytes:      u64,
}

pub fn write_main_event_csv(path: &str, rows: &[PhaseMetricRow]) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record(&[
        "runner", "phase", "phase_index", "ops_into_phase", "total_ops",
        "p50_us", "p99_us", "factor_utilization", "uaf", "memory_bytes", "storage_bytes",
    ])?;
    for r in rows {
        wtr.write_record(&[
            r.runner.clone(),
            r.phase.clone(),
            r.phase_index.to_string(),
            r.ops_into_phase.to_string(),
            r.total_ops.to_string(),
            r.p50_us.to_string(),
            r.p99_us.to_string(),
            r.factor_utilization.to_string(),
            r.uaf.to_string(),
            r.memory_bytes.to_string(),
            r.storage_bytes.to_string(),
        ])?;
    }
    wtr.flush()?;
    Ok(())
}

/// Plot factor utilization over time for multiple engines on one chart.
/// `series`: list of (runner_label, (total_ops, factor_utilization)) points.
pub fn plot_multi_engine_factor_util(
    path: &str,
    series: &[(&str, Vec<(usize, f64)>)],
) -> Result<()> {
    let root = BitMapBackend::new(path, (1280, 720)).into_drawing_area();
    root.fill(&WHITE)?;

    let max_x = series.iter()
        .flat_map(|(_, pts)| pts.iter().map(|(x, _)| *x))
        .max()
        .unwrap_or(1);

    let mut chart = ChartBuilder::on(&root)
        .caption("Factor Utilization Over Time — All Engines", ("sans-serif", 28))
        .margin(20)
        .x_label_area_size(50)
        .y_label_area_size(60)
        .build_cartesian_2d(0usize..max_x, 0.0f64..1.05f64)?;

    chart.configure_mesh()
        .x_desc("Total Operations")
        .y_desc("Factor Utilization")
        .draw()?;

    let palette: [RGBColor; 4] = [
        RGBColor(31, 119, 180),
        RGBColor(214, 39, 40),
        RGBColor(44, 160, 44),
        RGBColor(148, 103, 189),
    ];

    for (i, (label, points)) in series.iter().enumerate() {
        let color = &palette[i % palette.len()];
        chart.draw_series(LineSeries::new(points.iter().cloned(), color))?
            .label(*label)
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], *color));
    }

    chart.configure_series_labels()
        .background_style(&WHITE.mix(0.8))
        .border_style(&BLACK)
        .draw()?;
    root.present()?;
    Ok(())
}

/// Plot adaptation latency as a bar chart (one bar per phase per engine).
/// `bars`: list of (label, latency_ops_or_none) — None bars are drawn at max height with a marker.
pub fn plot_adaptation_latency(
    path: &str,
    bars: &[(&str, Option<usize>)],
    ops_per_phase: usize,
) -> Result<()> {
    let root = BitMapBackend::new(path, (1280, 480)).into_drawing_area();
    root.fill(&WHITE)?;

    let max_y = ops_per_phase;
    let n = bars.len();

    let mut chart = ChartBuilder::on(&root)
        .caption("Adaptation Latency Per Phase (ops to reach 90% factor utilization)", ("sans-serif", 22))
        .margin(20)
        .x_label_area_size(60)
        .y_label_area_size(70)
        .build_cartesian_2d(0usize..n, 0usize..max_y)?;

    chart.configure_mesh()
        .y_desc("Operations")
        .disable_x_mesh()
        .draw()?;

    for (i, (label, lat)) in bars.iter().enumerate() {
        let height = lat.unwrap_or(ops_per_phase);
        let color = if lat.is_none() { RED.mix(0.6) } else { BLUE.mix(0.7) };
        chart.draw_series(std::iter::once(Rectangle::new(
            [(i, 0), (i + 1, height)],
            color.filled(),
        )))?;
        let text_y = height.min(max_y.saturating_sub(max_y / 20));
        chart.draw_series(std::iter::once(Text::new(
            label.to_string(),
            (i, text_y),
            ("sans-serif", 12).into_font().color(&BLACK),
        )))?;
    }

    root.present()?;
    Ok(())
}

pub fn plot_factor_utilization_over_time(
    path: &str,
    snapshots: &[EvolutionPoint],
) -> Result<()> {
    let root = BitMapBackend::new(path, (1024, 768)).into_drawing_area();
    root.fill(&WHITE)?;

    let max_x = snapshots.iter().map(|s| s.operations).max().unwrap_or(1);
    let max_y = 1.0f64;

    let mut chart = ChartBuilder::on(&root)
        .caption("Factor Utilization Over Time", ("sans-serif", 30))
        .margin(20)
        .x_label_area_size(40)
        .y_label_area_size(60)
        .build_cartesian_2d(0usize..max_x, 0.0f64..max_y)?;

    chart.configure_mesh().draw()?;

    chart.draw_series(LineSeries::new(
        snapshots.iter().map(|s| (s.operations, s.factor_utilization)),
        &BLUE,
    ))?
    .label("Factor Utilization")
    .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &BLUE));

    chart.configure_series_labels().border_style(&BLACK).draw()?;
    root.present()?;
    Ok(())
}
