#![allow(clippy::print_literal)]
#![allow(clippy::needless_return)]
#![allow(dropping_references)]
#![allow(clippy::assertions_on_constants)]
mod util;

//note "_bm" are reserved label for labelling notes containing web page bookmarks
//note "_code" are reserved label for labelling notes containing ``` code blocks

//todo allow importing Chrome Bookmark Export HTML file into notes
//todo add feature of detecting and listing URL(s) in note. And user may select one of them to copy into clipboard

use crabrs::*;
use crabsqliters::*;

use log::*;

use std::collections::*;
use std::path::PathBuf;
use std::process::*;
use std::*;

use crate::ctxdef::*;
use crate::util::*;

#[macro_use(defer)]
extern crate scopeguard;

fn main() -> ExitCode {
    env::set_var("RUST_BACKTRACE", "1"); //? not 100% sure this has 0 impact on performance? Maybe setting via command line instead of hardcoding is better?
                                         //env::set_var("RUST_LIB_BACKTRACE", "1");//? this line is useless?
                                         ////
    env::set_var("RUST_LOG", "trace"); //note this line must be above logger init.
    env_logger::init();

    let args: Vec<String> = env::args().collect(); //Note that std::env::args will panic if any argument contains invalid Unicode.
    fn the_end() {
        if std::thread::panicking() {
            info!("{}", "PANICKING");
        }
        info!("{}", "FINISHED");
    }
    defer! {
        the_end();
    }
    if main_inner(args).is_err() {
        return ExitCode::from(1);
    }
    ExitCode::from(0)
}

fn cmd_exit(_: &mut Ctx) -> CustRes<bool> {
    return Ok(false);
}

fn cmd_reload(con: &mut Ctx) -> CustRes<bool> {
    con.chosen_dir.clear();
    con.chosen_lbl.clear();
    cmd_reset(con)?;
    {
        let db = &mut con.db;
        db.execute("delete from files", ())?;
        db.execute("delete from attachments", ())?;
        db.execute("delete from lbls", ())?;
    }
    read_notes(con)?;
    Ok(true)
}

fn cmd_cat(con: &mut Ctx) -> CustRes<bool> {
    let mut cached_stmt = con
        .db
        .prepare_cached("select content from files where tmpid=?1")?;
    let mut num_of_selected = 0;
    for (idx, tidraw) in con.def.filter_buf.iter().enumerate() {
        let tid: i64 = if *tidraw < 0 {
            num_of_selected += 1;
            -tidraw
        } else {
            continue;
        };
        println!();
        println!("{}{}{}", "[", idx, "]");
        let mut rows = cached_stmt.query((tid,))?;
        let row = rows.next()?.ok_or("TMPID unexpectedly invalid")?;
        let cont: String = row.get(0)?;
        coutln!(cont);
    }
    println!("{}{}{}", "******* ", num_of_selected, " SELECTED");
    return Ok(true);
}

fn cmd_list(con: &mut Ctx) -> CustRes<bool> {
    show_recs_in_filter_buf(con)?;
    return Ok(true);
}

fn cmd_lbl(con: &mut Ctx) -> CustRes<bool> {
    println!("{}{}", "Chosen label: ", con.def.chosen_lbl);
    return Ok(true);
}

fn cmd_lbls(con: &mut Ctx) -> CustRes<bool> {
    let mut lbl_distinct: Vec<String> = vec![];
    let mut cached_stmt = con.db.prepare_cached("select distinct lbl from lbls")?;
    let mut rows = cached_stmt.query([])?;
    while let Some(row) = rows.next()? {
        let lbl: String = row.get(0)?;
        lbl_distinct.push(lbl);
    }
    lbl_distinct.sort_unstable();
    println!("{:?}", lbl_distinct);
    Ok(true)
}

fn cmd_h(con: &mut Ctx) -> CustRes<bool> {
    //note print fn will print hex address. Useless for user.
    coutln!("List of simple cmds:");
    println!("{:?}", con.cmds.keys());
    coutln!("List of cmds with args:");
    println!("{:?}", con.cargs.keys());
    coutln!("List of cmds starting with special character:");
    println!(
        "{:?}",
        con.bargs
            .keys()
            .map(|&byt| byt as char)
            .collect::<Vec<char>>()
    );
    Ok(true)
}

fn cmd_dir(con: &mut Ctx) -> CustRes<bool> {
    let folders = get_all_folders(&con.db)?;
    for (idx, folder) in folders.iter().enumerate() {
        println!(
            "{}{} {}{}",
            if folder == &con.def.chosen_dir {
                '+'
            } else {
                ' '
            },
            idx,
            "/",
            folder
        );
    }
    cout_n_flush!("Choose one (or do nothing with empty input): ");
    let choice = match con.stdin_w.lines.next() {
        None => {
            coutln!("Input ended.");
            return Ok(false);
        }
        Some(Err(err)) => {
            let l_err: std::io::Error = err;
            return Err(l_err.into());
        }
        Some(Ok(linestr)) => linestr,
    };
    let chosen_idx = match choice.parse::<usize>() {
        Err(_) => {
            coutln!("Invalid index");
            return Ok(true);
        }
        Ok(l_idx) => l_idx,
    };
    let folder = match folders.into_iter().nth(chosen_idx) {
        None => {
            coutln!("Index too great");
            return Ok(true);
        }
        Some(folder) => folder,
    };
    con.def.chosen_dir = folder;
    Ok(true)
}

fn cmd_recent_notes(con: &mut Ctx) -> CustRes<bool> {
    let tids = {
        let mut cached_stmt = con
            .db
            .prepare_cached("select tmpid from files order by mtime desc limit 5")?;
        query_n_collect_into_vec_i64(cached_stmt.query([]))?
    };
    iter_rows_to_update_filter_buf(con, tids)?;
    Ok(true)
}

fn cmd_noop(_: &mut Ctx) -> CustRes<bool> {
    Ok(true)
}

fn lbl_add(con: &mut Ctx, l_lbl: &str) -> CustRes<()> {
    let tids = mk_vec_of_selection(con);
    if tids.is_empty() {
        return Ok(());
    }
    {
        let mut cached_stmt = con
            .db
            .prepare_cached("insert into lbls select ?1,?2 where not exists(select 1 from lbls where tmpid=?1 and lbl=?2)")?;
        for tid in &tids {
            let l_tid: i64 = *tid;
            cached_stmt.execute((l_tid, l_lbl))?;
        }
    }
    let mut pobj = PathBuf::new();
    for tid in &tids {
        pobj.clone_from(con.def.ndir());
        info_write_for_note(con, *tid, &mut pobj)?;
    }
    println!(
        "{}{}{}{}",
        l_lbl,
        ": insertion of label executed on ",
        tids.len(),
        " files."
    );
    Ok(())
}
fn lbl_remove(con: &mut Ctx, l_lbl: &str) -> CustRes<()> {
    let tids = mk_vec_of_selection(con);
    if tids.is_empty() {
        return Ok(());
    }
    {
        let mut cached_stmt = con
            .db
            .prepare_cached("delete from lbls where tmpid=?1 and lbl=?2")?;
        for tid in &tids {
            let l_tid: i64 = *tid;
            cached_stmt.execute((l_tid, l_lbl))?;
        }
    }
    let mut pobj = PathBuf::new();
    for tid in &tids {
        pobj.clone_from(con.def.ndir());
        info_write_for_note(con, *tid, &mut pobj)?;
    }
    println!(
        "{}{}{}{}",
        l_lbl,
        ": deletion of label executed on ",
        tids.len(),
        " files."
    );
    Ok(())
}

fn cmd_lbl_remove(con: &mut Ctx) -> CustRes<bool> {
    if con.def.chosen_lbl.is_empty() {
        println!("{}", "Chosen label is empty");
        return Ok(true);
    }
    let l_lbl = mem::take(&mut con.def.chosen_lbl);
    lbl_remove(con, &l_lbl)?;
    con.def.chosen_lbl = l_lbl;
    Ok(true)
}

fn cmd_lbl_add(con: &mut Ctx) -> CustRes<bool> {
    if con.def.chosen_lbl.is_empty() {
        println!("{}", "Chosen label is empty");
        return Ok(true);
    }
    let l_lbl = mem::take(&mut con.def.chosen_lbl);
    lbl_add(con, &l_lbl)?;
    con.def.chosen_lbl = l_lbl;
    Ok(true)
}

fn cmd_del(con: &mut Ctx) -> CustRes<bool> {
    let tids = mk_vec_of_selection(con);
    if tids.is_empty() {
        return Ok(true);
    }
    {
        let mut cached_stmt = con
            .db
            .prepare_cached("select rel from files where tmpid=?1")?;
        let mut pobj = PathBuf::new();
        for tid in &tids {
            let l_tid: i64 = *tid;
            let mut rows = cached_stmt.query((l_tid,))?;
            let row = rows.next()?.ok_or("TMPID unexpectedly invalid")?;
            let rel: String = row.get(0)?;
            pobj.clone_from(con.def.ndir());
            push_comps_to_pb(&mut pobj, rel);
            fs::remove_dir_all(&pobj)?;
            del_upper_dirs_if_useless(con, &mut pobj)?;
        }
    }
    exec_with_slice_i64(&con.db, "delete from files where tmpid=?1", &tids)?;
    exec_with_slice_i64(&con.db, "delete from attachments where tmpid=?1", &tids)?;
    exec_with_slice_i64(&con.db, "delete from lbls where tmpid=?1", &tids)?;
    exec_with_slice_i64(&con.db, "delete from filter_buf where tid=?1", &tids)?;
    con.def.filter_buf.retain(|elem| *elem >= 0);
    println!("{}", "DONE");
    Ok(true)
}

fn deselect(con: &mut Ctx) {
    for selid in &mut con.def.filter_buf {
        *selid = selid.abs();
    }
}
fn cmd_deselect(con: &mut Ctx) -> CustRes<bool> {
    deselect(con);
    Ok(true)
}

fn cmd_all(con: &mut Ctx) -> CustRes<bool> {
    for selid in &mut con.def.filter_buf {
        *selid = -selid.abs();
    }
    Ok(true)
}

fn cmd_reset(con: &mut Ctx) -> CustRes<bool> {
    clear_filter_buf(con)?;
    con.def.filter_buf.clear();
    Ok(true)
}

fn cmd_edit(con: &mut Ctx) -> CustRes<bool> {
    let mut tid = 0;
    for selid in &con.def.filter_buf {
        if *selid < 0 {
            if tid != 0 {
                coutln!("More than one note selected.");
                return Ok(true);
            }
            tid = -selid;
        }
    }
    if tid == 0 {
        coutln!("No note selected.");
        return Ok(true);
    }
    if con.def.editor_cmd.is_empty() {
        coutln!("No editor_cmd set");
        return Ok(true);
    }
    let mut cmd = Command::new(&con.def.editor_cmd[0]);
    if con.def.editor_cmd.len() > 1 {
        cmd.args(&con.def.editor_cmd[1..]);
    }
    //note using Stdio::null() to avoid editor output (usually stderr) appear on terminal of this program
    cmd.arg(mk_pathstr_for_note(con, tid)?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?; //todo no need to end program if spawn fails?
    Ok(true)
}

fn lbl_input(con: &mut Ctx) -> CustRes<bool> {
    let pat = con.def.ilinearg();
    if pat.is_empty() {
        println!("{}", "Cannot be empty.");
        return Ok(true);
    }
    if pat.starts_with('_') {
        println!(
            "{}",
            "Label cannot start with underscore. (Such labels are reserved for internal use.)"
        );
        return Ok(true);
    }
    con.def.chosen_lbl = pat;
    println!("{}{}", "Chosen: ", con.def.chosen_lbl);
    Ok(true)
}

fn lbls_search(con: &mut Ctx) -> CustRes<bool> {
    let pat = con.def.ilinearg();
    let mut lbl_distinct: Vec<String> = vec![];
    {
        let mut cached_stmt = con
            .db
            .prepare_cached("select distinct lbl from lbls where instr(lbl,?1)")?;
        let mut rows = cached_stmt.query((pat,))?;
        while let Some(row) = rows.next()? {
            let lbl: String = row.get(0)?;
            lbl_distinct.push(lbl);
        }
    }
    if lbl_distinct.is_empty() {
        println!("{}", "No match.");
        return Ok(true);
    }
    if lbl_distinct.len() == 1 {
        con.def.chosen_lbl = lbl_distinct.into_iter().next().unwrap();
        println!("{}{}", "Chosen: ", con.def.chosen_lbl);
        return Ok(true);
    }
    lbl_distinct.sort_unstable();
    for (idx, l_lbl) in lbl_distinct.iter().enumerate() {
        println!("{} {}", idx, l_lbl);
    }
    cout_n_flush!("Please choose: ");
    let choice: String = match con.stdin_w.lines.next() {
        None => {
            warn!("{}", "Unexpected stdin EOF");
            return Ok(false);
        }
        Some(Err(err)) => {
            let l_err: io::Error = err;
            return Err(l_err.into());
        }
        Some(Ok(linestr)) => linestr,
    };
    let idx: usize = match choice.parse::<usize>() {
        Err(_) => {
            println!("{}", "Invalid index");
            return Ok(true);
        }
        Ok(l_idx) => l_idx,
    };
    if let Some(sel_lbl) = lbl_distinct.into_iter().nth(idx) {
        con.def.chosen_lbl = sel_lbl;
        println!("{}{}", "Chosen: ", con.def.chosen_lbl);
    } else {
        println!("{}", "Invalid index");
    }
    Ok(true)
}
fn ca_where(con: &mut Ctx) -> CustRes<bool> {
    cout_n_flush!("ORDER BY (unordered if empty): ");
    let mut orderby: String;
    match con.stdin_w.lines.next() {
        None => {
            warn!("{}", "Unexpected stdin EOF");
            return Ok(false);
        }
        Some(Err(err)) => {
            let l_err: std::io::Error = err;
            return Err(l_err.into());
        }
        Some(Ok(linestr)) => {
            orderby = linestr;
        }
    }
    if !orderby.is_empty() {
        orderby.insert_str(0, " order by ");
    }
    let sqlstr = if con.filter_buf.is_empty() {
        "SELECT tmpid from files where ".to_owned() + &con.def.ilinearg() + &orderby
    } else {
        "SELECT tmpid from files where tmpid in(select tid from filter_buf) and (".to_owned()
            + &con.def.ilinearg()
            + ")"
            + &orderby
    };
    let tids = {
        let mut stmt = match con.db.prepare(&sqlstr) {
            Ok(statem) => statem,
            Err(err) => {
                warn!("{}{}", "Err during preparing stmt: ", err);
                return Ok(true);
            }
        };
        query_n_collect_into_vec_i64(stmt.query([]))?
    };
    iter_rows_to_update_filter_buf(con, tids)?;
    Ok(true)
}
fn ca_new(con: &mut Ctx) -> CustRes<bool> {
    let filenm = con.def.ilinearg();
    let fnms: Vec<_> = filenm.split(&['/', '\\']).collect();
    if fnms.is_empty() {
        coutln!("Path cannot be empty.");
        return Ok(true);
    }
    let mut dirp = mk_full_path_of_chosen_dir(con);
    for fnm in &fnms {
        if fnm.is_empty() {
            coutln!("Path component cannot be empty.");
            return Ok(true);
        }
        for ch in fnm.chars() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                continue;
            }
            coutln!("Path must consist of alphanumeric, undersocre, or hyphen.");
            return Ok(true);
        }
        if !fnm.as_bytes()[0].is_ascii_alphanumeric() {
            coutln!("First char must be alphanumeric.");
            return Ok(true);
        }
        if !fnm.as_bytes().last().unwrap().is_ascii_alphanumeric() {
            coutln!("Last char must be alphanumeric.");
            return Ok(true);
        }
        dirp.push(fnm);
        let conflict = dirp.join(INFO_FN);
        if real_reg_file_without_symlink(&conflict) {
            coutln!("Conflict found.");
            return Ok(true);
        }
    }
    if dirp.try_exists()? {
        coutln!("Folder already exists.");
        return Ok(true);
    }
    let tid = new_note(con, &mut dirp, fnms)?;
    deselect(con);
    con.def.filter_buf.push(-tid);
    exec_with_slice_i64(&con.db, "insert into filter_buf values(?1)", &[tid])?;
    Ok(true)
}
fn ca_dir(con: &mut Ctx) -> CustRes<bool> {
    let mut folders = get_all_folders(&con.db)?;
    let globstr = con.def.ilinearg();
    folders.retain(|k| k.contains(&globstr));
    if folders.is_empty() {
        coutln!("No match found");
        return Ok(true);
    }
    println!("{:?}", folders);
    if folders.len() > 1 {
        coutln!("Too many matches");
        return Ok(true);
    }
    con.def.chosen_dir = folders.into_iter().next().unwrap();
    Ok(true)
}

fn plus(con: &mut Ctx) -> CustRes<bool> {
    let lbl_distinct = get_single_lbl_match(con)?;
    if lbl_distinct.is_empty() {
        return Ok(true);
    }
    lbl_add(con, &lbl_distinct)?;
    Ok(true)
}
fn minus(con: &mut Ctx) -> CustRes<bool> {
    let lbl_distinct = get_single_lbl_match(con)?;
    if lbl_distinct.is_empty() {
        return Ok(true);
    }
    lbl_remove(con, &lbl_distinct)?;
    Ok(true)
}
fn ca_like(con: &mut Ctx) -> CustRes<bool> {
    let mut pat = con.def.ilinearg();
    if pat.is_empty() {
        coutln!("Cannot be empty.");
        return Ok(true);
    }
    macro_rules! selec {
        () => {
            "SELECT tmpid from files where "
        };
    }
    macro_rules! glob1 {
        () => ( "tmpid in(select tmpid from lbls where lbl like ?1) or tmpid in(select tmpid from attachments where rel like ?1) or rel like ?1 or content like ?1" )
    }
    //const : &'static str not working here with concat!. One way is to use macro_rules. Another way might be using const fn to do `+` (abandon concat!).
    let sqlstr = if con.filter_buf.is_empty() {
        concat!(selec!(), glob1!())
    } else {
        concat!(
            selec!(),
            "tmpid in(select tid from filter_buf) and (",
            glob1!(),
            ")"
        )
    };
    let tids = {
        let mut cached_stmt = con.db.prepare_cached(sqlstr)?;
        pat.insert(0, '%');
        pat.push('%');
        query_n_collect_into_vec_i64(cached_stmt.query((pat,)))?
    };
    iter_rows_to_update_filter_buf(con, tids)?;
    Ok(true)
}
fn slash(con: &mut Ctx) -> CustRes<bool> {
    let mut pat = con.def.ilinearg();
    if pat.is_empty() {
        coutln!("Cannot be empty.");
        return Ok(true);
    }
    macro_rules! selec {
        () => {
            "SELECT tmpid from files where "
        };
    }
    macro_rules! glob1 {
        () => ( "tmpid in(select tmpid from lbls where lbl glob ?1) or tmpid in(select tmpid from attachments where rel glob ?1) or rel glob ?1 or content glob ?1" )
    }
    //const : &'static str not working here with concat!. One way is to use macro_rules. Another way might be using const fn to do `+` (abandon concat!).
    let sqlstr = if con.filter_buf.is_empty() {
        concat!(selec!(), glob1!())
    } else {
        concat!(
            selec!(),
            "tmpid in(select tid from filter_buf) and (",
            glob1!(),
            ")"
        )
    };
    let tids = {
        let mut cached_stmt = con.db.prepare_cached(sqlstr)?;
        pat.push('*');
        query_n_collect_into_vec_i64(cached_stmt.query((pat,)))?
    };
    iter_rows_to_update_filter_buf(con, tids)?;
    Ok(true)
}
fn get_single_lbl_match(con: &mut Ctx) -> CustRes<String> {
    let pat = con.def.ilinearg();
    let mut lbl_distinct: Vec<String> = vec![];
    {
        let mut cached_stmt = con
            .db
            .prepare_cached("select distinct lbl from lbls where instr(lbl,?1)")?;
        let mut rows = cached_stmt.query((pat,))?;
        while let Some(row) = rows.next()? {
            let lbl: String = row.get(0)?;
            lbl_distinct.push(lbl);
        }
    }
    if lbl_distinct.len() != 1 {
        println!("{}{:?}", "Must match exactly 1 label. ", lbl_distinct);
        return Ok("".to_owned());
    }
    Ok(lbl_distinct.into_iter().next().unwrap())
}
fn iter_rows_to_update_filter_buf(con: &mut Ctx, tids: Vec<i64>) -> Result<(), CustomErr> {
    if tids.is_empty() {
        println!("{}", "Nothing found. Filtering is cancelled.");
    } else {
        clear_filter_buf(con)?;
        exec_with_slice_i64(&con.db, "insert into filter_buf values(?1)", &tids)?;
        con.def.filter_buf = tids;
        show_recs_in_filter_buf(con)?;
    }
    Ok(())
}
fn show_recs_in_filter_buf(con: &Ctx) -> Result<(), CustomErr> {
    let mut cached_stmt = con.db.prepare_cached("with a(label)as(select group_concat(lbl) from lbls where tmpid=?1)select label,files.size,mtime,files.rel,attachments.rel,attachments.size from files left join a on 1=1 left join attachments on files.tmpid=attachments.tmpid where files.tmpid=?1")?;
    let mut num_of_selected = 0;
    for (idx, tidraw) in con.def.filter_buf.iter().enumerate() {
        println!();
        print!("{}{}{}", "[", idx, "]");
        let tid: i64 = if *tidraw < 0 {
            num_of_selected += 1;
            println!("{}", " +");
            -tidraw
        } else {
            println!();
            *tidraw
        };
        let mut rows = cached_stmt.query((tid,))?;
        let mut printed_once = false;
        while let Some(row) = rows.next()? {
            fn show_field(pre: &str, fstr: String) {
                if !fstr.is_empty() {
                    println!("{} {}", pre, fstr);
                }
            }
            if !printed_once {
                printed_once = true;
                let lbls: Option<String> = row.get(0)?;
                let flen: i64 = row.get(1)?;
                let ms: i64 = row.get(2)?;
                let noterel: String = row.get(3)?;
                coutln!(millis2display(ms));
                coutln!(noterel);
                println!("{} {}", "SIZE", flen);
                show_field("LABEL", lbls.unwrap_or_default());
            }
            let attrel: Option<String> = row.get(4)?;
            let attsize: Option<i64> = row.get(5)?;
            show_field("ATTREL", attrel.unwrap_or_default());
            if let Some(inner) = attsize {
                println!("{} {}", "ATTSIZE", inner);
            }
        }
    }
    println!("{}{}{}", "******* ", num_of_selected, " SELECTED");
    Ok(())
}

type TupStrFnCtx = (String, fn(&mut Ctx) -> CustRes<bool>);
type TupU8FnCtx = (u8, fn(&mut Ctx) -> CustRes<bool>);

fn main_inner(args: Vec<String>) -> CustRes<()> {
    let db = rusqlite::Connection::open_in_memory()?;
    let cmds_arr: [TupStrFnCtx; 21] = [
        ("reload".to_owned(), cmd_reload),
        ("exit".to_owned(), cmd_exit),
        ("quit".to_owned(), cmd_exit),
        ("h".to_owned(), cmd_h),
        ("help".to_owned(), cmd_h),
        ("l".to_owned(), cmd_list),
        ("list".to_owned(), cmd_list),
        ("lbl".to_owned(), cmd_lbl),
        ("lbls".to_owned(), cmd_lbls),
        ("dir".to_owned(), cmd_dir),
        ("e".to_owned(), cmd_edit),
        ("edit".to_owned(), cmd_edit),
        ("reset".to_owned(), cmd_reset),
        ("+".to_owned(), cmd_lbl_add),
        ("-".to_owned(), cmd_lbl_remove),
        ("".to_owned(), cmd_noop),
        ("recent".to_owned(), cmd_recent_notes),
        ("del".to_owned(), cmd_del),
        ("all".to_owned(), cmd_all),
        ("deselect".to_owned(), cmd_deselect),
        ("cat".to_owned(), cmd_cat),
    ];
    let cargs_arr: [TupStrFnCtx; 6] = [
        ("where".to_owned(), ca_where),
        ("new".to_owned(), ca_new),
        ("lbl".to_owned(), lbl_input),
        ("lbls".to_owned(), lbls_search),
        ("dir".to_owned(), ca_dir),
        ("like".to_owned(), ca_like),
    ];
    let bargs_arr: [TupU8FnCtx; 3] = [(b'/', slash), (b'+', plus), (b'-', minus)];
    //todo add user-customized command alias from config file (e.g. maybe ll for label, d for dir)
    let home_dir = dirs::home_dir().ok_or("Failed to get home directory.")?;
    if !real_dir_without_symlink(&home_dir) {
        return dummy_err("Failed to recognize the home dir as folder.");
    }
    let mut ctx = Ctx {
        args,
        def: CtxDef::init(home_dir),
        db,
        cmds: BTreeMap::<String, fn(&mut Ctx) -> CustRes<bool>>::from(cmds_arr),
        cargs: BTreeMap::<String, fn(&mut Ctx) -> CustRes<bool>>::from(cargs_arr),
        bargs: BTreeMap::<u8, fn(&mut Ctx) -> CustRes<bool>>::from(bargs_arr),
    };
    //note cmds must contain "" to make sure it is handled so that other commands can safely assume the input has at least one u8
    debug_assert!(ctx.cmds.contains_key(""));
    fs::create_dir_all(ctx.def.ndir())?;
    //*** BEGIN editor_cmd config
    let ecp = ctx.def.app_support_dir.join("editor_cmd");
    if real_reg_file_without_symlink(&ecp) {
        let coll: Vec<String> = fs::read_to_string(ecp)?
            .lines()
            .filter(|elem| !elem.is_empty())
            .map(|elem| elem.to_owned())
            .collect();
        if coll.is_empty() {
            coutln!("No command found in editor_cmd.");
        } else {
            println!("{}{:?}", "Editor command: ", coll);
            ctx.def.editor_cmd = coll;
        }
    } else {
        coutln!("No command for editor_cmd");
    }
    //*** END editor_cmd config
    //note no need to set chosen_dir. It should just be empty, which represents note_dir.
    //ctx.def.chosen_dir = ctx
    //    .def
    //    .note_dir
    //    .to_str()
    //    .ok_or("Failed to conv note dir to str")?
    //    .to_owned();
    //fixme instead of waiting indefinitely for file lock, just try and tell user if they want to use READ-ONLY mode
    let flock = monitor_enter(&ctx.def.lock_p)?;
    defer! {
        monitor_exit(flock);
    }
    init_db(&ctx.db)?;
    read_notes(&mut ctx)?;
    loop {
        cout_n_flush!(">>> ");
        ctx.def.iline = match ctx.stdin_w.lines.next() {
            None => {
                coutln!("Input ended.");
                break Ok(());
            }
            Some(Err(err)) => {
                let l_err: std::io::Error = err;
                break Err(l_err.into());
            }
            Some(Ok(linestr)) => linestr,
        };
        let cb = ctx.cmds.get(&ctx.def.iline);
        match cb {
            None => {}
            Some(inner) => {
                if inner(&mut ctx)? {
                    continue;
                } else {
                    break Ok(());
                }
            }
        }
        debug_assert!(!ctx.def.iline.is_empty());
        match ctx.bargs.get(&ctx.def.iline.as_bytes()[0]) {
            None => {}
            Some(inner) => {
                ctx.def.iline_argidx = 1;
                if inner(&mut ctx)? {
                    continue;
                } else {
                    break Ok(());
                }
            }
        }
        if let Some(sidx) = ctx.iline.find(' ') {
            let cb = ctx.cargs.get(&ctx.def.iline[..sidx]);
            match cb {
                None => {}
                Some(inner) => {
                    ctx.def.iline_argidx = sidx + 1;
                    if inner(&mut ctx)? {
                        continue;
                    } else {
                        break Ok(());
                    }
                }
            }
        }
        if ctx.iline.bytes().all(|c| c.is_ascii_digit()) {
            let idx: usize = match ctx.iline.parse::<usize>() {
                Err(_) => {
                    println!("{}", "Invalid index");
                    continue;
                }
                Ok(inner) => inner,
            };
            if let Some(selid) = ctx.def.filter_buf.get_mut(idx) {
                *selid *= -1;
                show_recs_in_filter_buf(&ctx)?;
            } else {
                println!("{}", "Invalid index");
            }
        } else {
            warn!("{}", "Command not recognized.");
        }
    }
}

//todo you need inotify/ file system watcher to monitor text change under all notes. So that when using external editor, the changes will immediately be searchable in this program.

fn read_notes(con: &mut Ctx) -> Result<(), CustomErr> {
    let mut tmpid: i64 = 0;
    let db = &mut con.db;
    //note when this var drops, it calls roolback by default. (Unless consumed via `commit`)
    let tx = db.transaction()?;
    //firstly for each dir chk info file. If info exists it means a note file. Otherwise it is subdir.
    let rd = fs::read_dir(con.def.ndir())?;
    //con.def.note_dir_len = con
    //    .def
    //    .ndir()
    //    .to_str()
    //    .ok_or("Note dir cannot conv to str")?
    //    .len()
    //    + 1;
    let mut iters = Vec::<std::fs::ReadDir>::new();
    iters.push(rd);
    let mut path_components: Vec<String> = vec![];
    loop {
        match iters.last_mut().unwrap().next() {
            None => {
                iters.pop();
                if iters.is_empty() {
                    break;
                }
                path_components.pop();
            }
            Some(Err(err)) => {
                return Err(err.into());
            }
            Some(Ok(dirent)) => {
                let fpath = dirent.path();
                let filety = dirent.file_type()?; //docs:"will not traverse symlinks"
                if !filety.is_dir() {
                    warn!("{}{:?}", "Unexpected file found and IGNORED: ", fpath);
                    warn!("{}", "NOTE this file will be FORCEFULLY DELETED after certain operations (the timing is indeterminate) are performed by this program.");
                    continue;
                }
                path_components.push(dirent.file_name().into_string()?);
                let mut jobj = match read_info_if_it_exists(&fpath)? {
                    None => {
                        iters.push(fs::read_dir(&fpath)?);
                        continue;
                    }
                    Some(inner) => inner,
                };
                jobj.rel = path_components.join("/");
                //jobj.rel = fpath.into_os_string().into_string()?[con.def.note_dir_len..].to_owned();
                path_components.pop();
                tmpid += 1;
                {
                    tx.prepare_cached("insert into files values(?1, ?2, ?3, ?4, ?5)")?
                        .execute((tmpid, jobj.size, jobj.mtime, jobj.rel, jobj.content))?;
                }
                for attached in jobj.att {
                    tx.prepare_cached("insert into attachments values(?1, ?2, ?3)")?
                        .execute((tmpid, attached.0, attached.1))?;
                }
                for lbl in jobj.lbls {
                    tx.prepare_cached("insert into lbls values(?1, ?2)")?
                        .execute((tmpid, lbl))?;
                }
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn read_note(folder: &path::Path, jobj: &mut InfoJsonElem) -> CustRes<()> {
    let npath = folder.join("note");
    let apath = folder.join("att");
    if !real_reg_file_without_symlink(&npath) {
        error!("{}{:?}", "Corruption of note found: ", npath);
        return dummy_err("Corruption error");
    }
    let md = fs::metadata(&npath)?;
    jobj.size = md.len() as i64;
    jobj.mtime = systemtime2millis(md.modified()?);
    jobj.content = fs::read_to_string(&npath)?;
    if jobj.content.contains("http://") || jobj.content.contains("https://") {
        jobj.lbls.push("_bm".to_owned());
    }
    if jobj.content.split(['\n', '\r']).any(|r| r == "```") {
        jobj.lbls.push("_code".to_owned());
    }
    if !apath.try_exists()? {
        return Ok(());
    }
    if !real_dir_without_symlink(&apath) {
        return dummy_err("Attachment folder is corrupted");
    }
    //let apath = apath.into_os_string().into_string()?;
    let apathlen = apath.to_str().ok_or("Failed to conv path to str")?.len();
    for dirent in walkdir::WalkDir::new(apath) {
        let dr = dirent?;
        let md = dr.metadata()?;
        if !md.is_file() {
            continue;
        }
        let mut pstr: String = dr.into_path().into_os_string().into_string()?;
        //note pstr here is really longest FULL path
        pstr = pstr[apathlen + 1..].to_owned();
        jobj.att.push((pstr, md.len() as i64));
    }
    Ok(())
}

fn read_info_if_it_exists(folder: &path::Path) -> CustRes<Option<InfoJsonElem>> {
    let mpath = folder.join(INFO_FN);
    if !real_reg_file_without_symlink(&mpath) {
        return Ok(None);
    }
    let mut retval = info_read(&mpath)?;
    read_note(folder, &mut retval)?;
    Ok(Some(retval))
}

fn init_db(conn: &rusqlite::Connection) -> Result<(), CustomErr> {
    conn.execute(
        "CREATE TABLE lbls (
	    tmpid integer not null,
	    lbl text not null
        )",
        (),
    )?;
    conn.execute("CREATE INDEX idx_tmpid_lbls ON lbls (tmpid)", ())?;
    conn.execute("CREATE INDEX idx_lbl ON lbls (lbl)", ())?;
    conn.execute(
        "CREATE TABLE files (
	    tmpid INTEGER PRIMARY KEY,
	    size integer not null,
	    mtime integer not null,
	    rel text not null,
	    content text not null
        )",
        (),
    )?;
    conn.execute("CREATE INDEX idx_size ON files (size)", ())?;
    conn.execute("CREATE INDEX idx_mtime ON files (mtime)", ())?;
    conn.execute(
        "CREATE TABLE attachments (
	    tmpid integer not null,
	    rel text not null,
	    size integer not null
        )",
        (),
    )?;
    conn.execute("CREATE INDEX idx_tmpid_att ON attachments (tmpid)", ())?;
    conn.execute("CREATE INDEX idx_size_att ON attachments (size)", ())?;
    conn.execute(
        "CREATE TABLE filter_buf (
	    tid INTEGER PRIMARY KEY
        )",
        (),
    )?;
    Ok(())
}

fn get_avail_tmpid(conn: &rusqlite::Connection) -> Result<i64, CustomErr> {
    let mut cached_stmt =
        conn.prepare_cached("select tmpid from files order by tmpid desc limit 1")?;
    let mut rows = cached_stmt.query([])?;
    if let Some(row) = rows.next()? {
        let retval: i64 = row.get(0)?;
        return Ok(retval + 1);
    } else {
        return Ok(1);
    }
}

fn monitor_enter(lock_p: &path::Path) -> CustRes<fs::File> {
    file_lock(lock_p, b"\n")
}
fn monitor_exit(fobj: fs::File) -> bool {
    file_unlock(fobj)
}

pub struct Ctx {
    args: Vec<String>,
    def: CtxDef,
    db: rusqlite::Connection,
    cmds: BTreeMap<String, fn(&mut Ctx) -> CustRes<bool>>,
    cargs: BTreeMap<String, fn(&mut Ctx) -> CustRes<bool>>,
    bargs: BTreeMap<u8, fn(&mut Ctx) -> CustRes<bool>>,
}
impl ops::Deref for Ctx {
    type Target = CtxDef;

    fn deref(&self) -> &CtxDef {
        &self.def
    }
}
impl ops::DerefMut for Ctx {
    fn deref_mut(&mut self) -> &mut CtxDef {
        &mut self.def
    }
}

//making this mod block is just for making some fields private, e.g. note_dir (so that you can rest assured you do not mistakingly modify it. It is like final/readonly in Java/C#)
mod ctxdef {
    use crabrs::*;
    use std::path::PathBuf;
    use std::*;

    #[derive(Default)]
    pub struct CtxDef {
        pub stdin_w: StdinWrapper,
        home_dir: PathBuf,
        everycom: PathBuf,
        pub app_support_dir: PathBuf,
        note_dir: PathBuf,
        pub lock_p: PathBuf,
        //pub note_dir_len: usize,
        //readonly: bool, //todo add feature of readonly mode (e.g. when lock file is locked by another process)
        pub iline: String,
        pub iline_argidx: usize,
        pub editor_cmd: Vec<String>,
        pub chosen_dir: String,
        pub chosen_lbl: String,
        //chosen_note: InfoJsonElem,
        pub filter_buf: Vec<i64>,
    }

    const PKG_NAME: &str = env!("CARGO_PKG_NAME");
    const _: () = assert!(!PKG_NAME.is_empty(), "Constraint on const");

    impl CtxDef {
        pub fn init(home_dir: PathBuf) -> Self {
            let mut retval = CtxDef {
                home_dir,
                ..Default::default()
            };
            retval.everycom = retval.home_dir.join(".everycom");
            retval.app_support_dir = retval.everycom.join(PKG_NAME);
            retval.lock_p = retval.app_support_dir.join("lock");
            retval.note_dir = retval.app_support_dir.join("note");
            retval
        }
        pub fn ndir(&self) -> &PathBuf {
            &self.note_dir
        }
        pub fn ilinearg(&mut self) -> String {
            mem::take(&mut self.iline)[self.iline_argidx..].to_owned()
        }
    }
}

fn mk_full_path_of_chosen_dir(con: &Ctx) -> PathBuf {
    let mut retval = con.ndir().clone();
    if con.chosen_dir.is_empty() {
        return retval;
    }
    for comp in con.chosen_dir.split('/') {
        retval.push(comp);
    }
    retval
}

fn clear_filter_buf(con: &Ctx) -> CustRes<()> {
    let mut cached_stmt = con.db.prepare_cached("delete from filter_buf")?;
    cached_stmt.execute([])?;
    Ok(())
}

fn mk_vec_of_selection(con: &Ctx) -> Vec<i64> {
    let mut tids: Vec<i64> = vec![];
    for selid in &con.def.filter_buf {
        if *selid < 0 {
            tids.push(-selid);
        }
    }
    if tids.is_empty() {
        println!("{}", "No selection");
    }
    tids
}
