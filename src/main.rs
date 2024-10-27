#![warn(unused_crate_dependencies)]
use anyhow::Context;
use chrono::{DateTime, FixedOffset, Local, NaiveDateTime};
use git2::{Oid, Repository, Signature, Time};
use globset::GlobMatcher;
use lazy_static::lazy_static;
use masof::{KeyCode, KeyMap, Renderer, Stylize};
use std::collections::{btree_map, BTreeMap, HashMap};
use std::collections::HashSet;
use std::io::{BufWriter, Write};
use std::process::{Command, Stdio};
use std::rc::Rc;
use structopt::StructOpt;
use futures::FutureExt;
use futures::StreamExt;

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
    Io(#[from] std::io::Error),

    #[error("Masof error: {0}")]
    MasofRendrer(#[from] masof::renderer::Error),
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

    /// Reverse the display order
    #[structopt(name = "interactive", long, short = "i")]
    interactive: bool,

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

fn print_time(w: &mut impl Write, time: &Time, index: usize) -> std::io::Result<()> {
    let dt = DateTime::<Local>::from_utc(
        NaiveDateTime::from_timestamp_opt(time.seconds(), 0).expect("invalid timstamp"),
        FixedOffset::east_opt(0).unwrap(),
    );

    write!(w,
        "{} {}",
        if index % 2 == 0 {
            RGB(255, 200, 0)
        } else {
            RGB((255 as u16 * 3 / 5) as u8, (200 as u16 * 3 / 5) as u8, 0)
        }
        .paint(format!("{}", dt.format("%Y.%m.%d %H:%M:%S"))),
        White.bold().paint(format!("| ")),
    )?;

    Ok(())
}

fn print_commit(
    w: &mut impl Write,
    idx: usize,
    _repo: &Repository,
    time: &Time,
    msg: &String,
    id_revs: &Vec<(&Oid, &HashSet<Rc<String>>)>,
    oid_to_diff_id: &HashMap<&Oid, String>,
    highlight: &Option<String>,
    branches: &Vec<Rc<String>>,
    colors: &Vec<Colour>,
    variants: bool,
) -> std::io::Result<()> {
    match highlight {
        Some(highlight) if !msg.contains(highlight) => {
            return Ok(());
        }
        _ => {}
    }

    let mut contained_in = HashSet::new();
    for (_, c_revs) in id_revs {
        contained_in = contained_in.union(c_revs).cloned().collect();
    }

    for (oid, c_revs) in id_revs {
        print_time(w, &time, idx)?;

        let diff_id = oid_to_diff_id.get(oid).map(|x| x.as_str()).unwrap_or("");

        for (i, item) in branches.iter().enumerate() {
            let revs = if variants {
                c_revs
            } else {
                &contained_in
            };

            if revs.contains(item) {
                write!(w, "{}", colors[i % colors.len()].paint(format!("x")))?;
            } else {
                write!(w, "{}", colors[i % colors.len()].paint(format!("┊")))?;
            }
        }

        write!(w, " ")?;
        write!(w, "{}", &oid.to_string()[..12])?;
        if variants {
            if id_revs.len() > 1 {
                write!(w, " {}", RGB(100, 100, 100).paint(&diff_id[..8]))?;
            } else {
                write!(w, " {}", "        ")?;
            }
        }
        write!(w, " {}", White.bold().paint(msg))?;

        writeln!(w)?;

        if !variants {
            break;
        }
    }

    return Ok(());
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
enum MainAction {
    Quit,
    ItemDown,
    ItemUp,
    ItemPageDown,
    ItemPageUp,
    FirstItem,
    LastItem,
}

struct Printer<'a> {
    args: Args,
    repo: git2::Repository,
    colors: Vec<Colour>,
    branches: Vec<Rc<String>>,
    oid_to_diff_id: HashMap<&'a Oid, String>,
    main_mode_map: KeyMap<MainAction>,
    commits: Vec<(Time, String, Vec<(&'a Oid, &'a HashSet<Rc<String>>)>)>,
}

impl<'a> Printer<'a> {
    fn print_commits(&self, w: &mut impl Write) -> std::io::Result<()> {
        if self.args.reverse {
            for (idx, (timestamp, msg, id_revs)) in self.commits.iter().rev().enumerate() {
                print_commit(
                    w,
                    idx,
                    &self.repo,
                    &timestamp,
                    &msg,
                    &id_revs,
                    &self.oid_to_diff_id,
                    &self.args.search,
                    &self.branches,
                    &self.colors,
                    self.args.variants,
                )?;
            }
        } else {
            for (idx, (timestamp, msg, id_revs)) in self.commits.iter().enumerate() {
                print_commit(
                    w,
                    idx,
                    &self.repo,
                    &timestamp,
                    &msg,
                    &id_revs,
                    &self.oid_to_diff_id,
                    &self.args.search,
                    &self.branches,
                    &self.colors,
                    self.args.variants,
                )?;
            }
        }

        return Ok(());
    }

    fn print_branches(&self, w: &mut impl Write) -> std::io::Result<Vec<Rc<String>>>  {
        let mut display_order = vec![];
        if self.args.reverse {
            for (i, name) in self.branches.iter().enumerate() {
                self.print_branch(w, i, &*name)?;
                display_order.push(name.clone());
            }
        } else {
            for (i, name) in self.branches.iter().enumerate().rev() {
                self.print_branch(w, i, &*name)?;
                display_order.push(name.clone());
            }
        }

        Ok(display_order)
    }

    fn print_branch(&self, w: &mut impl Write, i: usize, name: &str) -> std::io::Result<()>  {
        let prefix = " ".repeat(22);

        write!(w, "{}", prefix)?;
        for c in 0..i {
            write!(w, "{}", self.colors[c % self.colors.len()].paint(format!("│")))?;
        }
        writeln!(w,
            "{}",
            self.colors[i % self.colors.len()].paint(format!("{}", name))
        )?;

        Ok(())
    }

    fn print_sep(&self, w: &mut impl Write) -> std::io::Result<()> {
        let prefix = " ".repeat(22);

        write!(w, "{}", prefix)?;

        for c in 0..self.branches.len() {
            write!(w, "{}", self.colors[c % self.colors.len()].paint(format!("│")))?;
        }

        writeln!(w, "")?;

        Ok(())
    }

    fn run(self) -> Result<(), Error> {
        if self.args.interactive {
            return tokio::runtime::Builder::new_current_thread()
                .enable_all().build()?
                .block_on(async move { self.interactive().await });
        }

        let w = &mut std::io::stdout();
        if self.args.reverse {
            self.print_branches(w)?;
            self.print_sep(w)?;
            self.print_commits(w)?;
        } else {
            self.print_commits(w)?;
            self.print_sep(w)?;
            self.print_branches(w)?;
        }

        Ok(())
    }

    async fn interactive(mut self) -> Result<(), Error> {
        let mut renderer = Renderer::default();
        let stdout = &mut std::io::stdout();

        let m = &mut self.main_mode_map;
        m.add_no_mods(KeyCode::Char('q'), MainAction::Quit);
        m.add_no_mods(KeyCode::Up, MainAction::ItemUp);
        m.add_no_mods(KeyCode::Down, MainAction::ItemDown);
        m.add_no_mods(KeyCode::PageUp, MainAction::ItemPageUp);
        m.add_no_mods(KeyCode::PageDown, MainAction::ItemPageDown);
        m.add_no_mods(KeyCode::Home, MainAction::FirstItem);
        m.add_no_mods(KeyCode::End, MainAction::LastItem);

        renderer.term_on(stdout)?;
        let r = self.event_loop(stdout, &mut renderer).await;
        renderer.term_off(stdout)?;

        r
    }

    async fn event_loop<'b, 'c: 'b + 'a>(&'c mut self, stdout: &'b mut std::io::Stdout, renderer: &'b mut Renderer) -> Result<(), Error> {
        let mut reader = crossterm::event::EventStream::new();

        let mut interactive_mode = InteractiveMode{
            stdout,
            renderer,
            leave: false,
            main: self,
            selected_item: 0,
            view_offset: 0,
            view_size: 0,
            nr_items: 0,
        };

        interactive_mode.redraw()?;

        while !interactive_mode.leave {
            futures::select! {
                maybe_event = reader.next().fuse() => {
                    match maybe_event {
                        Some(Ok(masof::Event::Mouse{..})) => continue,
                        Some(Ok(event)) => {
                            interactive_mode.renderer.event(&event);
                            interactive_mode.on_event(event)?
                        }
                        Some(Err(_)) => {
                            break;
                        }
                        None => {}
                    }
                }
            };

            interactive_mode.redraw()?;
        }

        Ok(())
    }
}

struct InteractiveMode<'a, 'b: 'a> {
    stdout: &'a mut std::io::Stdout,
    renderer: &'a mut Renderer,
    leave: bool,
    main: &'b mut Printer<'b>,
    selected_item: usize,
    view_offset: usize,
    nr_items: usize,
    view_size: usize,
}

impl<'a, 'b: 'a> InteractiveMode<'a, 'b> {
    fn on_event(&mut self, event: crossterm::event::Event) -> Result<(), Error> {
        match event {
            crossterm::event::Event::Key(event) => {
                let action = self.main.main_mode_map.get_action(event).map(|x| x.clone());
                if let Some(action) = action {
                    match action {
                        MainAction::Quit => {
                            self.leave = true;
                        }
                        MainAction::ItemDown => {
                            self.selected_item += 1;
                        },
                        MainAction::ItemUp => {
                            self.selected_item = self.selected_item.saturating_sub(1);
                        },
                        MainAction::ItemPageDown => {
                            self.selected_item += self.view_size - 1;
                        },
                        MainAction::ItemPageUp => {
                            if self.view_size > 0 {
                                self.selected_item = self.selected_item.saturating_sub(self.view_size - 1);
                            }
                        },
                        MainAction::FirstItem => {
                            self.selected_item = 0;
                        },
                        MainAction::LastItem => {
                            self.selected_item = self.nr_items - 1;
                        },
                    }
                }
            }
            _ => {

            }
        }

        if self.nr_items > 0 && self.selected_item >= self.nr_items {
            self.selected_item = self.nr_items - 1;
        }
        if self.selected_item >= self.view_offset + self.view_size {
            self.view_offset = self.selected_item - self.view_size + 1;
        }
        if self.selected_item < self.view_offset {
            self.view_offset = self.selected_item;
        }

        Ok(())
    }

    fn redraw(&mut self) -> Result<(), Error> {
        self.renderer.begin()?;

        let mut buf = BufWriter::new(Vec::new());
        self.main.print_sep(&mut buf)?;
        let display_branches = self.main.print_branches(&mut buf)?;
        let bytes = buf.into_inner().unwrap();
        let string = String::from_utf8(bytes).unwrap();
        let cs = masof::ContentStyle::new()
            .with(masof::Color::Rgb { r: 255, g: 255, b: 255 });
        let screen_height = self.renderer.height() as usize;

        let branches_y = screen_height - (display_branches.len() + 1);
        for (idx, line) in string.lines().enumerate() {
            if idx >= screen_height {
                break;
            }

            let y = branches_y + idx;
            self.renderer.draw_raw_ansi(0, y as u16, line, cs);
            // if idx == self.selected_item {
            //     self.renderer.with_cell(0, y as u16, self.renderer.width(), |_, cs| {
            //         *cs = cs.on(masof::Color::Rgb{r: 0, g: 70, b: 120});
            //     })
            // }
        }


        let mut buf = BufWriter::new(Vec::new());
        self.main.print_commits(&mut buf)?;
        let bytes = buf.into_inner().unwrap();
        let string = String::from_utf8(bytes).unwrap();
        let cs = masof::ContentStyle::new()
            .with(masof::Color::Rgb { r: 255, g: 255, b: 255 });

        let max_commits_view = branches_y;
        self.view_size = max_commits_view;
        self.nr_items = self.main.commits.len();
        for (idx, line) in string.lines().enumerate() {

            if idx < self.view_offset {
                continue;
            }
            if idx >= self.view_offset + self.view_size {
                break;
            }

            let y = idx - self.view_offset;
            self.renderer.draw_raw_ansi(0, y as u16, line, cs);
            if idx == self.selected_item {
                self.renderer.with_cell(0, y as u16, self.renderer.width(), |_, cs| {
                    *cs = cs.on(masof::Color::Rgb{r: 0, g: 70, b: 120});
                })
            }
        }

        self.renderer.end(self.stdout)?;

        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::from_args();
    let path = args.git_dir.as_ref().map(|s| &s[..]).unwrap_or(".");
    let repo = Repository::open(path).context("error opening repository")?;
    let ref_max_age = std::time::Duration::from_secs(86400 * args.days);
    let commit_max_age = std::time::Duration::from_secs(86400 * args.days);
    let mut author = args.author.clone();

    let config = repo.config().context("error reading repo configuration")?;
    if author.is_none() {
        let name = config.get_entry("user.name").context("error reading user.name git config")?;
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
            BranchKind::Glob(globset::Glob::new(&glob)
                .with_context(|| format!("error parsing glob pattern {}", glob))?.compile_matcher())
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
    for refe in repo.references().context("no references")? {
        if let Some(refname) = refe.context("error parsing ref")?.name() {
            let st = if let Some(caps) = RE_BRANCH.captures(&refname) {
                caps.get(1).unwrap().as_str().to_owned()
            } else {
                continue;
            };
            let revspec = repo.revparse(&refname)
                .with_context(|| format!("rev parse error: {}", refname))?;
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
                    let output = output.with_context(|| format!("output error for refscript: {}", refscript))?;
                    let lines: Vec<_> = output.lines().collect();
                    if lines.len() >= 2 {
                        let name = lines[0];
                        let inp = lines[1];
                        let revspec = repo.revparse(&inp)
                            .with_context(|| format!("rev parse error for refscript output {:?}, full output {:?}", inp, output))?;
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
        let revwalk = revwalk.with_hide_callback(&callback).context("with_hide_callback failed")?;

        for commit in revwalk {
            let commit = commit.context("no commit in revwalk")?;
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
    let mut oid_to_diff_id = HashMap::new();
    for (_, _, id_revs) in &v {
        for (oid, c_revs) in id_revs {
            unsorted_branches = unsorted_branches.union(c_revs).cloned().collect();

            let diff_id = String::from_utf8(
                Command::new("sh")
                .arg("-c")
                .arg(&format!("git show {oid} --format= | cat | sed 's/^@@.*/@@/g' | sed 's/^index.*//' | sha1sum -"))
                .stdout(Stdio::piped())
                .output()
                .expect("failed executing 'git show'").stdout)
                .expect("utf-8 conversion");

            oid_to_diff_id.insert(*oid, diff_id);
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

    let printer = Printer {
        args,
        repo,
        colors,
        branches,
        oid_to_diff_id,
        main_mode_map: KeyMap::new(),
        commits: v,
    };

    printer.run()?;

    Ok(())
}
