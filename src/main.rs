use clap::Parser;
use colored::Colorize;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;
use mapping::MappingKind;
use once_cell::sync::Lazy;
use rand::{seq::SliceRandom, thread_rng};
use std::path::PathBuf;
use task::split_tasks;

mod anvil;
mod mapping;
mod nbt;
mod remap;
mod task;
mod text;

static MULTI: Lazy<MultiProgress> = Lazy::new(MultiProgress::new);

#[derive(Debug, Parser)]
struct Cli {
    /// The path to the world
    path: PathBuf,
    /// The kind of mapping
    mapping_kind: MappingKind,
    /// The path to the mapping file
    mapping_file: PathBuf,
    /// The number of threads to use
    #[clap(short, long, default_value = "24")]
    threads: usize,
    /// Skip the confirmation
    #[clap(short, long)]
    yes: bool,
    /// Do not modify the world
    #[clap(short, long)]
    no: bool,
}

fn main() {
    let cli = Cli::parse();

    let logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    LogWrapper::new((*MULTI).clone(), logger)
        .try_init()
        .unwrap();

    if std::mem::size_of::<usize>() < 8 {
        log::error!(
            "usize is less than 64-bit, you may encounter integer overflow when \
        dealing with some malformed NBT"
        );
        log::error!("Do not report this issue to the author, as it is not worth fixing");
        log::error!(
            "Since Minecraft almost can't run on 32-bit devices, \
        running this program, which is designed to work with Minecraft, is meaningless"
        );
    }

    let path = cli.path;
    let tasks = task::scan_world(&path);
    let Ok(mut tasks) = tasks else {
        log::error!("Failed to scan world: {:#?}", tasks);
        return;
    };
    log::info!("{} files found in {}", tasks.len(), path.display());
    let mapping = match mapping::get_mapping(cli.mapping_kind, &cli.mapping_file) {
        Ok(m) => m,
        Err(err) => {
            log::error!("Failed to load mapping: {:#?}", err);
            return;
        }
    };
    if mapping.is_empty() {
        log::warn!("Empty mapping");
        log::warn!("The program will do identity mapping, i.e. f(x) = x");
        log::warn!("This is only used for testing the program on your world");
    }
    log::info!("{}", "Task Summary".bold().underline());
    log::info!("{}", "Files:".yellow());
    for task in &tasks {
        log::info!("   {}", task.display());
    }
    log::info!("{}", "Mapping:".yellow());
    for (k, v) in &mapping {
        log::info!("   {} -> {}", k, v);
    }
    log::info!(
        "{} {} {} {}",
        "We will modify".red(),
        tasks.len(),
        "files in the world at".red(),
        path.display()
    );
    log::info!(
        "{}",
        "Make sure to backup your world before running this program".red()
    );
    log::info!("{}", "Is this correct? [YES/NO/Y/N]".green().bold());
    if cli.no {
        log::info!("{}", "Nothing to do!".red());
        return;
    } else if cli.yes {
        log::info!("{}", "YES".green());
    } else {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        if input.trim().to_lowercase() != "yes" && input.trim().to_lowercase() != "y" {
            log::error!("Cancelled by user");
            return;
        }
    }

    tasks.shuffle(&mut thread_rng());
    let mut handles = vec![];
    for (i, thread_task) in split_tasks(&tasks, cli.threads).iter().enumerate() {
        let pg = MULTI.add(indicatif::ProgressBar::new(tasks.len() as u64));
        let template = format!("worker-{:02}: ", i) + "[{bar:60.cyan/blue}] {pos}/{len} {msg} ";
        pg.set_style(
            indicatif::ProgressStyle::default_bar()
                .template(&template)
                .unwrap()
                .progress_chars("#>-"),
        );
        handles.push(task::run_tasks(
            path.clone(),
            unsafe { std::mem::transmute(*thread_task) },
            pg,
            unsafe { std::mem::transmute(&mapping) },
        ));
    }

    let mut stat = 0;
    for handle in handles {
        stat += handle.join().unwrap();
    }
    log::info!(
        "{} {} {}",
        "Done!".green().bold(),
        stat,
        "uuid fields are modified".green().bold()
    );
}
