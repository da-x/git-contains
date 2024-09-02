#![warn(unused_crate_dependencies)]
use chrono::{DateTime, FixedOffset, Local, NaiveDateTime};
use git2::{Oid, Repository, Signature, Time};
use globset::GlobMatcher;
use lazy_static::lazy_static;
use std::collections::{BTreeMap, btree_map};
use std::collections::HashSet;
use std::process::{Command, Stdio};
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

    #[error("Io error: {0}")]
    Io(std::io::Error),
}

#[derive(StructOpt, Clone)]
struct Args {
    /// Alternative git directory to use
    git_dir: Option<String>,

    /// Don't show commits older than this
    #[structopt(name = "days", long, short = "d", default_value = "30")]
    days: u64,

    /// Reverse the display order
    #[structopt(name = "reverse", long, short = "r")]
    reverse: bool,

    #[structopt(name = "author", long)]
    /// Author to sort by, defaults to current user name
    author: Option<String>,

    #[structopt(name = "branch", long)]
    /// Branches to show, or `<refscript>:<param>` triggers
    branches: Vec<String>,

    /// Only show commits having this text in commit message
    #[structopt(name = "search", long)]
    search: Option<String>,

    /// Show all the variants of commits having the same commit subject line
    #[structopt(name = "variants", long, short = "v")]
    variants: bool,
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
    id_revs: &Vec<(&Oid, &HashSet<Rc<String>>)>,
    highlight: &Option<String>,
    branches: &Vec<Rc<String>>,
    colors: &Vec<Colour>,
    variants: bool,
) {
    match highlight {
        Some(highlight) if !msg.contains(highlight) => {
            return;
        }
        _ => {}
    }

    let mut contained_in = HashSet::new();
    for (_, c_revs) in id_revs {
        contained_in = contained_in.union(c_revs).cloned().collect();
    }

    for (oid, c_revs) in id_revs {
        print_time(&time, idx);

        let diff_id = String::from_utf8(
            Command::new("sh")
            .arg("-c")
            .arg(&format!("git show {oid} --format= | cat | sed 's/^@@.*/@@/g' | sed 's/^index.*//' | sha1sum -"))
            .stdout(Stdio::piped())
            .output()
            .expect("failed executing 'git show'").stdout)
            .expect("utf-8 conversion");

        for (i, item) in branches.iter().enumerate() {
            let revs = if variants {
                c_revs
            } else {
                &contained_in
            };

            if revs.contains(item) {
                print!("{}", colors[i % colors.len()].paint(format!("x")));
            } else {
                print!("{}", colors[i % colors.len()].paint(format!("┊")));
            }
        }

        print!(" ");
        print!("{}", &oid.to_string()[..12]);
        if variants {
            if id_revs.len() > 1 {
                print!(" {}", RGB(100, 100, 100).paint(&diff_id[..8]));
            } else {
                print!(" {}", "        ");
            }
        }
        print!(" {}", White.bold().paint(msg));

        println!();

        if !variants {
            break;
        }
    }
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
                    self.args.variants,
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
                    self.args.variants,
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

fn main() -> anyhow::Result<()> {
    let args = Args::from_args();
    let path = args.git_dir.as_ref().map(|s| &s[..]).unwrap_or(".");
    let repo = Repository::open(path)?;
    let ref_max_age = std::time::Duration::from_secs(86400 * args.days);
    let commit_max_age = std::time::Duration::from_secs(86400 * args.days);
    let mut author = args.author.clone();

    let config = repo.config()?;
    if author.is_none() {
        let name = config.get_entry("user.name")?;
        let name = name.value();
        author = name.map(|x| x.to_owned());
    }

    let mut refscript = None;
    if let Ok(entry) = config.get_entry("contains.refscript") {
        refscript = entry.value().map(|x| x.to_owned());
    }

    let mut branch_infos = vec![];

    enum BranchKind {
        Glob(GlobMatcher),
        RefScript(String),
    }

    struct BranchInfo {
        show_if_empty: bool,
        kind: BranchKind,
    }

    for (idx, glob) in args.branches.iter().enumerate() {
        let mut show_if_empty = false;
        let glob = if glob.starts_with("!") {
            show_if_empty = true;
            &glob[1..]
        } else {
            glob
        };
        let kind = if glob.contains(":") {
            BranchKind::RefScript(glob.to_owned())
        } else {
            BranchKind::Glob(globset::Glob::new(&glob)?.compile_matcher())
        };
        let item = BranchInfo {
            show_if_empty,
            kind,
        };

        branch_infos.push((idx, item));
    }

    lazy_static! {
        static ref RE_BRANCH: regex::Regex =
            regex::Regex::new("^refs/remotes/origin/(.+)$").unwrap();
    }

    // Which commits OIDs in which branches
    let mut mapoid_to_branches = BTreeMap::new();
    let mut found_branches = BTreeMap::new();

    let mut branches = vec![];
    for refe in repo.references()? {
        if let Some(refname) = refe?.name() {
            let st = if let Some(caps) = RE_BRANCH.captures(&refname) {
                caps.get(1).unwrap().as_str().to_owned()
            } else {
                continue;
            };
            let revspec = repo.revparse(&refname)?;
            let mut matched = None;
            let mut show_if_empty = false;

            for (idx, branch_info) in branch_infos.iter() {
                match &branch_info.kind {
                    BranchKind::Glob(glob) => {
                        if glob.is_match(&st) {
                            matched = Some(idx);
                            show_if_empty = branch_info.show_if_empty;
                            break;
                        }
                    },
                    BranchKind::RefScript(_) => {
                    },
                }
            }

            let idx = if let Some(idx) = matched {
                idx
            } else {
                continue;
            };

            found_branches.insert(st.to_owned().clone(), (idx, show_if_empty));

            let name = Rc::new(format!("{}", st));
            branches.push((name.to_owned(), revspec.from().unwrap().id()));
        }
    }

    if let Some(refscript) = refscript {
        for (idx, branch_info) in branch_infos.iter() {
            match &branch_info.kind {
                BranchKind::Glob(_) => {},
                BranchKind::RefScript(input) => {
                    let refscript = if let Ok(home) = std::env::var("HOME") {
                        refscript.replace("${HOME}", home.as_str())
                    } else {
                        refscript.clone()
                    };
                    let output = String::from_utf8(std::process::Command::new(&refscript)
                        .arg(input).output()?.stdout);
                    let output = output?;
                    let lines: Vec<_> = output.lines().collect();
                    if lines.len() >= 2 {
                        let name = lines[0];
                        let revspec = repo.revparse(&lines[1])?;
                        let st = name;
                        let oid = revspec.from().unwrap().id();

                        found_branches.insert(st.to_owned().clone(), (idx, branch_info.show_if_empty));
                        branches.push((Rc::new(name.to_owned()), oid));
                    }
                },
            }
        }
    }

    for (name, oid) in branches {
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
                btree_map::Entry::Vacant(v) => v.insert(HashSet::new()),
                btree_map::Entry::Occupied(o) => o.into_mut(),
            };

            item.insert(name.clone());
        }
    }

    // Which commit messages map to what OIDs, skipping merges
    let mut msg_map = BTreeMap::new();
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
                btree_map::Entry::Vacant(v) => v.insert((committer.when(), Vec::new())),
                btree_map::Entry::Occupied(o) => o.into_mut(),
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
    for (name, (_, show_if_empty)) in found_branches.iter() {
        if *show_if_empty {
            branches.push((None, Rc::new(name.to_owned())));
        }
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
