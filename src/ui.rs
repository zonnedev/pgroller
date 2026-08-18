use std::time::{Duration, Instant};

use colored::Colorize;
use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

// ─── Spinner Styles ──────────────────────────────────────────────────────────

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg}")
        .unwrap()
        .tick_strings(SPINNER_FRAMES)
}

fn progress_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} [{bar:30.cyan/dim}] {pos}/{len} {msg}")
        .unwrap()
        .tick_strings(SPINNER_FRAMES)
        .progress_chars("━╸─")
}

// ─── Banner ──────────────────────────────────────────────────────────────────

pub fn print_banner() {
    println!();
    println!(
        "  {} {} {}",
        style("⟳").cyan().bold(),
        style("pgroller").cyan().bold(),
        style("v0.1.0").dim()
    );
    println!("  {}", style("migration rollback validator").dim());
}

// ─── Phase Headers ───────────────────────────────────────────────────────────

pub fn print_phase(phase: &str) {
    println!();
    println!("  {} {}", style("▸").cyan().bold(), style(phase).bold());
}

pub fn print_subphase(msg: &str) {
    println!("    {} {}", style("│").dim(), style(msg).dim());
}

pub fn print_info(key: &str, value: &str) {
    println!(
        "    {} {}: {}",
        style("│").dim(),
        style(key).dim(),
        style(value).white()
    );
}

// ─── Progress Tracker ────────────────────────────────────────────────────────

pub struct TestProgress {
    multi: MultiProgress,
    overall: ProgressBar,
    current: Option<ProgressBar>,
    start_time: Instant,
}

impl TestProgress {
    pub fn new(total_migrations: usize) -> Self {
        let multi = MultiProgress::new();

        let overall = multi.add(ProgressBar::new(total_migrations as u64));
        overall.set_style(progress_style());
        overall.set_message("migrations");
        overall.enable_steady_tick(Duration::from_millis(80));

        Self {
            multi,
            overall,
            current: None,
            start_time: Instant::now(),
        }
    }

    pub fn start_migration(&mut self, version: u64, description: &str) {
        if let Some(prev) = self.current.take() {
            prev.finish_and_clear();
        }

        let pb = self.multi.add(ProgressBar::new_spinner());
        pb.set_style(spinner_style());
        pb.set_message(format!(
            "{}__{}",
            style(version).cyan().bold(),
            style(description).white()
        ));
        pb.enable_steady_tick(Duration::from_millis(80));
        self.current = Some(pb);
    }

    pub fn step(&self, msg: &str) {
        if let Some(pb) = &self.current {
            pb.set_message(format!("{}", style(msg).dim()));
        }
    }

    pub fn step_detail(&self, version: u64, description: &str, step: &str) {
        if let Some(pb) = &self.current {
            pb.set_message(format!(
                "{}__{} → {}",
                style(version).cyan().bold(),
                style(description).white(),
                style(step).dim()
            ));
        }
    }

    pub fn finish_migration_pass(&mut self, version: u64, description: &str, covered: usize) {
        if let Some(pb) = self.current.take() {
            let detail = if covered == 0 {
                "round-trip clean".to_string()
            } else {
                format!("{} annotated diffs", covered)
            };
            pb.finish_with_message(format!(
                "{} {}__{} — {}",
                "✓".green().bold(),
                style(version).green(),
                style(description).green(),
                style(detail).dim()
            ));
        }
        self.overall.inc(1);
    }

    pub fn finish_migration_fail(&mut self, version: u64, description: &str, uncovered: usize) {
        if let Some(pb) = self.current.take() {
            pb.finish_with_message(format!(
                "{} {}__{} — {} {}",
                "✗".red().bold(),
                style(version).red(),
                style(description).red(),
                style(format!("{} uncovered diffs", uncovered)).red(),
                style("[rollback]").red().dim()
            ));
        }
        self.overall.inc(1);
    }

    pub fn finish_migration_warning(&mut self, version: u64, description: &str, stale: usize) {
        if let Some(pb) = self.current.take() {
            pb.finish_with_message(format!(
                "{} {}__{} — {}",
                "⚠".yellow().bold(),
                style(version).yellow(),
                style(description).yellow(),
                style(format!("{} stale annotations", stale)).yellow()
            ));
        }
        self.overall.inc(1);
    }

    pub fn finish_migration_error(&mut self, version: u64, description: &str, error: &str) {
        if let Some(pb) = self.current.take() {
            pb.finish_with_message(format!(
                "{} {}__{} — {}",
                "✗".red().bold(),
                style(version).red().bold(),
                style(description).red(),
                style(format!("error: {}", error)).red().dim()
            ));
        }
        self.overall.inc(1);
    }

    pub fn finish_all(&self) {
        self.overall.finish_and_clear();
    }

    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
}

// ─── Final Report ────────────────────────────────────────────────────────────

pub fn print_separator() {
    println!();
    println!("  {}", style("─".repeat(50)).dim());
}

pub fn print_uncovered_details(
    version: u64,
    description: &str,
    uncovered: &[(String, String)], // (description, suggestion)
) {
    println!();
    println!(
        "  {} {}__{} — uncovered diffs:",
        "✗".red().bold(),
        style(version).red(),
        style(description).red(),
    );
    for (desc, suggestion) in uncovered {
        println!();
        println!("    {} {}", style("╭─").red().dim(), style(desc).red());
        println!(
            "    {} {}",
            style("╰→").cyan().dim(),
            style(suggestion).cyan().dim()
        );
    }
}

pub fn print_stale_details(
    version: u64,
    description: &str,
    stale: &[(String, String, String)], // (annotation_text, name, reason)
) {
    println!();
    println!(
        "  {} {}__{} — stale annotations (no matching diff):",
        "⚠".yellow().bold(),
        style(version).yellow(),
        style(description).yellow(),
    );
    for (annotation_text, _name, _reason) in stale {
        println!(
            "    {} {}",
            style("•").yellow(),
            style(annotation_text).yellow()
        );
    }
}

pub fn print_error_details(version: u64, description: &str, error: &str) {
    println!();
    println!(
        "  {} {}__{} — error:",
        "✗".red().bold(),
        style(version).red(),
        style(description).red(),
    );
    println!("    {} {}", style("╭─").red().dim(), style(error).red());
}

pub fn print_summary(
    total: usize,
    passed: usize,
    warnings: usize,
    test_failures: usize,
    production_errors: usize,
    elapsed: Duration,
) {
    println!();

    let elapsed_str = format_duration(elapsed);
    let _has_errors = production_errors > 0 || test_failures > 0;

    let mut parts: Vec<String> = vec![format!("{} passed", style(passed).green().bold())];

    if warnings > 0 {
        parts.push(format!("{} warnings", style(warnings).yellow().bold()));
    }

    if test_failures > 0 {
        parts.push(format!(
            "{} test failures",
            style(test_failures).red().bold()
        ));
    }

    if production_errors > 0 {
        parts.push(format!(
            "{} broken migrations",
            style(production_errors).red().bold()
        ));
    }

    let line = format!(
        "  {} {} migrations: {}  {}",
        if production_errors > 0 {
            style("●").red().to_string()
        } else if test_failures > 0 {
            style("●").red().to_string()
        } else if warnings > 0 {
            style("●").yellow().to_string()
        } else {
            style("●").green().to_string()
        },
        total,
        parts.join(", "),
        style(format!("({})", elapsed_str)).dim(),
    );

    println!("{}", line);
    println!();
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let millis = d.subsec_millis();

    if secs == 0 {
        format!("{}ms", millis)
    } else if secs < 60 {
        format!("{}.{:02}s", secs, millis / 10)
    } else {
        let mins = secs / 60;
        let remaining_secs = secs % 60;
        format!("{}m {}s", mins, remaining_secs)
    }
}
