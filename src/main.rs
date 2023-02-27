#![warn(unused_crate_dependencies)]
use chrono::{DateTime, FixedOffset, Local, NaiveDateTime};
use git2::{Oid, Repository, Signature, Time};
use lazy_static::lazy_static;
use std::collections::hash_map;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use structopt::StructOpt;

use ansi_term::Colour;
use ansi_term::Colour::{White, RGB};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Globset error; {0}")]
    GlobSet(#[from] globset::Error),

    #[error("Git error; {0}")]
    Git(#[from] git2::Error),

    #[error("Io error; {0}")]
    Io(std::io::Error),
}

#[derive(StructOpt, Clone)]
struct Args {
    /// Alternative git directory to use
    git_dir: Option<String>,

    /// Alternative git directory to use
    #[structopt(name = "days", long, short = "d", default_value = "30")]
    days: u64,

    /// Reverse the display order
    #[structopt(name = "reverse", long, short = "r")]
    reverse: bool,

    #[structopt(name = "author", long)]
    /// Author to sort by
    author: Option<String>,

    #[structopt(name = "branch", long)]
    /// Branches to show
    branches: Vec<String>,

    /// Highlight certain commits containing given text
    #[structopt(name = "search", long)]
    search: Option<String>,
}

fn sig_matches(sig: &Signature, arg: &Option<String>) -> bool {
    match *arg {
        Some(ref s) => {
            sig.name().map(|n| n.contains(s)).unwrap_or(false)
                || sig.email().map(|n| n.contains(s)).unwrap_or(false)
        }
        None => true,
    }
}

fn print_time(time: &Time, index: usize) {
    let dt = DateTime::<Local>::from_utc(
        NaiveDateTime::from_timestamp_opt(time.seconds(), 0).expect("invalid timstamp"),
        FixedOffset::east_opt(0).unwrap(),
    );

    print!(
        "{} {}",
        if index % 2 == 0 {
            RGB(255, 200, 0)
        } else {
            RGB((255 as u16 * 3 / 5) as u8, (200 as u16 * 3 / 5) as u8, 0)
        }
        .paint(format!("{}", dt.format("%Y.%m.%d %H:%M:%S"))),
        White.bold().paint(format!("| ")),
    );
}

fn print_commit(
    idx: usize,
    _repo: &Repository,
    time: &Time,
    msg: &String,
    c: &Vec<(&Oid, &HashSet<Rc<String>>)>,
    highlight: &Option<String>,
    branches: &Vec<Rc<String>>,
    colors: &Vec<Colour>,
) {
    match highlight {
        Some(highlight) if !msg.contains(highlight) => {
            return;
        }
        _ => {}
    }

    let mut contained_in = HashSet::new();
    for (_oid, c_revs) in c {
        contained_in = contained_in.union(c_revs).cloned().collect();
    }

    print_time(&time, idx);

    for (i, item) in branches.iter().enumerate() {
        if contained_in.contains(item) {
            print!("{}", colors[i % colors.len()].paint(format!("x")));
        } else {
            print!("{}", colors[i % colors.len()].paint(format!("┊")));
        }
    }

    print!(" ");
    print!(" {}", msg);

    println!();
}

struct Printer<'a> {
    args: Args,
    repo: git2::Repository,
    colors: Vec<Colour>,
    branches: Vec<Rc<String>>,
    v: Vec<(Time, String, Vec<(&'a Oid, &'a HashSet<Rc<String>>)>)>,
}

impl<'a> Printer<'a> {
    fn print_commits(&self) {
        if self.args.reverse {
            for (idx, (timestamp, msg, id_revs)) in self.v.iter().rev().enumerate() {
                print_commit(
                    idx,
                    &self.repo,
                    &timestamp,
                    &msg,
                    &id_revs,
                    &self.args.search,
                    &self.branches,
                    &self.colors,
                );
            }
        } else {
            for (idx, (timestamp, msg, id_revs)) in self.v.iter().enumerate() {
                print_commit(
                    idx,
                    &self.repo,
                    &timestamp,
                    &msg,
                    &id_revs,
                    &self.args.search,
                    &self.branches,
                    &self.colors,
                );
            }
        }
    }

    fn print_branches(&self) {
        if self.args.reverse {
            for (i, name) in self.branches.iter().enumerate() {
                self.print_branch(i, &*name);
            }
        } else {
            for (i, name) in self.branches.iter().enumerate().rev() {
                self.print_branch(i, &*name);
            }
        }
    }

    fn print_branch(&self, i: usize, name: &str) {
        let prefix = " ".repeat(22);

        print!("{}", prefix);
        for c in 0..i {
            print!("{}", self.colors[c % self.colors.len()].paint(format!("│")));
        }
        println!(
            "{}",
            self.colors[i % self.colors.len()].paint(format!("{}", name))
        );
    }

    fn print_sep(&self) {
        let prefix = " ".repeat(22);

        print!("{}", prefix);

        for c in 0..self.branches.len() {
            print!("{}", self.colors[c % self.colors.len()].paint(format!("│")));
        }

        println!("");
    }

    fn print(&self) -> Result<(), Error> {
        if self.args.reverse {
            self.print_branches();
            self.print_sep();
            self.print_commits();
        } else {
            self.print_commits();
            self.print_sep();
            self.print_branches();
        }

        Ok(())
    }
}

fn main() {
    match main_wrap() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(-1);
        }
    }
}

fn main_wrap() -> Result<(), Error> {
    let args = Args::from_args();
    let path = args.git_dir.as_ref().map(|s| &s[..]).unwrap_or(".");
    let repo = Repository::open(path)?;
    let ref_max_age = std::time::Duration::from_secs(86400 * args.days);
    let commit_max_age = std::time::Duration::from_secs(86400 * args.days);
    let mut author = args.author.clone();

    if author.is_none() {
        let config = repo.config()?;
        let name = config.get_entry("user.name")?;
        let name = name.value();
        author = name.map(|x| x.to_owned());
    }

    let mut patterns = vec![];

    for (idx, glob) in args.branches.iter().enumerate() {
        patterns.push((idx, globset::Glob::new(&glob)?.compile_matcher()));
    }

    lazy_static! {
        static ref RE_BRANCH: regex::Regex =
            regex::Regex::new("^refs/remotes/origin/(.+)$").unwrap();
    }

    // Which commits OIDs in which barnches
    let mut mapoid_to_branches = HashMap::new();
    let mut found_branches = HashMap::new();

    for refe in repo.references()? {
        if let Some(refname) = refe?.name() {
            let refname = &refname;

            let st = if let Some(caps) = RE_BRANCH.captures(&refname) {
                caps.get(1).unwrap().as_str()
            } else {
                continue;
            };

            let mut matched = None;
            for (idx, pattern) in patterns.iter() {
                if pattern.is_match(&st) {
                    matched = Some(idx);
                    break;
                }
            }

            let idx = if let Some(idx) = matched {
                idx
            } else {
                continue;
            };

            found_branches.insert(st.to_owned().clone(), idx);

            let revspec = repo.revparse(&refname)?;
            let name = Rc::new(format!("{}", st));
            let oid = revspec.from().unwrap().id();

            if let Ok(commit) = repo.find_commit(oid) {
                let time = commit.committer().when();
                let time = std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(time.seconds() as u64);
                if time.elapsed().unwrap() > ref_max_age {
                    continue;
                }
            }

            let mut revwalk = repo.revwalk()?;
            revwalk.push(oid)?;

            let callback = |cb| {
                if let Ok(commit) = repo.find_commit(cb) {
                    let time = commit.committer().when();
                    let time = std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(time.seconds() as u64);
                    if time.elapsed().unwrap() > commit_max_age {
                        return true;
                    }
                }
                false
            };
            let revwalk = revwalk.with_hide_callback(&callback)?;

            for commit in revwalk {
                let commit = commit?;
                let item = match mapoid_to_branches.entry(commit) {
                    hash_map::Entry::Vacant(v) => v.insert(HashSet::new()),
                    hash_map::Entry::Occupied(o) => o.into_mut(),
                };

                item.insert(name.clone());
            }
        }
    }

    // Which commit messages map to what OIDs, skipping merges
    let mut msg_map = HashMap::new();
    for (id, revs) in &mapoid_to_branches {
        let commit = repo.find_commit(*id)?;
        if commit.parents().len() > 1 {
            continue;
        }

        let committer = commit.committer();
        if !sig_matches(&commit.author(), &author) {
            continue;
        }

        let time = commit.committer().when();
        let time = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(time.seconds() as u64);
        if time.elapsed().unwrap() > commit_max_age {
            continue;
        }

        for msg in String::from_utf8_lossy(commit.message_bytes()).lines() {
            let item = match msg_map.entry(String::from(msg)) {
                hash_map::Entry::Vacant(v) => v.insert((committer.when(), Vec::new())),
                hash_map::Entry::Occupied(o) => o.into_mut(),
            };
            item.1.push((id, revs));
            break;
        }
    }

    let mut v = vec![];
    for (msg, (when, id_revs)) in msg_map {
        v.push((when, msg, id_revs));
    }

    v.sort_by(|y, x| y.0.cmp(&x.0));

    let mut unsorted_branches = HashSet::new();
    for (_, _, id_revs) in &v {
        for (_oid, c_revs) in id_revs {
            unsorted_branches = unsorted_branches.union(c_revs).cloned().collect();
        }
    }

    let mut branches = vec![];
    for branch in unsorted_branches.into_iter() {
        branches.push((found_branches.get(&*branch).map(|x| *x), branch));
    }
    branches.sort();
    let branches: Vec<_> = branches.into_iter().map(|x| x.1).collect();

    let mut colors = vec![];
    let m = 2;
    let n = 100;
    for r in 0..=m {
        for g in 0..=m {
            for b in 0..=m {
                let t = 255 - n;
                colors.push(RGB(n + (t * r) / m, n + (t * g) / m, n + (t * b) / m));
            }
        }
    }

    Printer {
        args,
        repo,
        colors,
        branches,
        v,
    }
    .print()?;

    Ok(())
}
