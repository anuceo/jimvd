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
