use std::{
    cell::Cell,
    collections::HashMap,
    path::{Path, PathBuf},
    thread::JoinHandle,
};

use indicatif::ProgressBar;
use uuid::Uuid;

use crate::remap::{remap_file, require_remapping};

pub fn run_tasks(
    world: PathBuf,
    tasks: &'static [PathBuf],
    pg: ProgressBar,
    mapping: &'static HashMap<Uuid, Uuid>,
) -> JoinHandle<usize> {
    std::thread::spawn(move || {
        pg.set_length(tasks.len() as u64);
        let stat = Cell::new(0);
        for task in tasks {
            pg.set_message(task.display().to_string());
            let cb = |uuid| {
                let ret = mapping.get(&uuid).copied();
                if ret.is_some() {
                    stat.set(stat.get() + 1);
                }
                ret
            };
            if let Err(err) = remap_file(&world, task, &cb) {
                log::error!("Failed to remap file {}: {:#?}", task.display(), err);
            };
            pg.inc(1);
        }
        stat.get()
    })
}

pub fn scan_world(world: &PathBuf) -> anyhow::Result<Vec<PathBuf>> {
    fn dfs_scan(
        world: &PathBuf,
        buf: &mut PathBuf,
        tasks: &mut Vec<PathBuf>,
        depth: usize,
    ) -> anyhow::Result<()> {
        if depth > 20 {
            return Ok(());
        }
        for entry in std::fs::read_dir(&*buf)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                buf.push(path.file_name().unwrap());
                dfs_scan(world, buf, tasks, depth + 1)?;
                buf.pop();
            } else {
                fn relative_path(world: &Path, path: &Path) -> PathBuf {
                    let p = path.strip_prefix(world).unwrap().to_path_buf();
                    if p.starts_with(std::path::MAIN_SEPARATOR.to_string()) {
                        p.strip_prefix(std::path::MAIN_SEPARATOR.to_string())
                            .unwrap()
                            .to_path_buf()
                    } else {
                        p
                    }
                }
                if require_remapping(&path) {
                    tasks.push(relative_path(world, &path));
                }
            }
        }
        Ok(())
    }
    let mut tasks = Vec::new();
    dfs_scan(world, &mut world.clone(), &mut tasks, 0)?;
    Ok(tasks)
}

pub fn split_tasks(tasks: &[PathBuf], count: usize) -> Vec<&[PathBuf]> {
    let mut ret = vec![];
    let block_size = tasks.len() / count;
    let block_remain = tasks.len() % count;
    let mut start = 0;
    for i in 0..count {
        let len = block_size + if i < block_remain { 1 } else { 0 };
        ret.push(&tasks[start..start + len]);
        start += len;
    }
    ret
}

#[cfg(test)]
#[test]
fn test() {
    let tasks = vec![
        PathBuf::from("a"),
        PathBuf::from("b"),
        PathBuf::from("c"),
        PathBuf::from("d"),
        PathBuf::from("e"),
        PathBuf::from("f"),
        PathBuf::from("g"),
        PathBuf::from("h"),
        PathBuf::from("i"),
        PathBuf::from("j"),
    ];
    assert_eq!(
        split_tasks(&tasks, 3)
            .iter()
            .map(|x| x.len())
            .collect::<Vec<_>>(),
        vec![4, 3, 3]
    );
    assert_eq!(
        split_tasks(&tasks, 4)
            .iter()
            .map(|x| x.len())
            .collect::<Vec<_>>(),
        vec![3, 3, 2, 2]
    );
}
