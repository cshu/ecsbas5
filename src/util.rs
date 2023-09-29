use crabrs::*;
use crabsqliters::*;
use log::*;
use serde::{Deserialize, Serialize};
//use std::io::Write;
use std::collections::*;
use std::path::PathBuf;
use std::*;

pub const INFO_FN: &'static str = "info";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct InfoJsonElem {
    #[serde(skip)]
    pub att: Vec<(String, i64)>,
    #[serde(skip)]
    pub tmpid: i64,
    #[serde(skip)]
    pub size: i64,
    #[serde(skip)]
    pub rel: String,
    #[serde(skip)]
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub lbls: Vec<String>,
    #[serde(skip)]
    pub mtime: i64,
    //#[serde(default)]
    //pub mtime: i64, //note this is the earliest time you are sure since when it has never been modifed, not physical mtime
    //#[serde(default)]
    //pub hash: String,
}

pub fn info_read(pathbuild: &path::Path) -> CustRes<InfoJsonElem> {
    use std::fs::*;
    let fhnd = File::open(pathbuild)?;
    let reader = std::io::BufReader::new(fhnd);
    let retval: InfoJsonElem = serde_json::from_reader(reader)?;
    Ok(retval)
}

fn info_write(pathbuild: &path::Path, elem: &InfoJsonElem) -> CustRes<()> {
    use std::fs::*;
    let mut file = File::create(pathbuild)?;
    serde_json::to_writer_pretty(&file, elem)?;
    use std::io::prelude::*;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn info_write_for_note(
    con: &mut super::Ctx,
    tid: i64,
    note_dir_clone: &mut PathBuf,
) -> CustRes<()> {
    let mut lbls = {
        let mut cached_stmt = con
            .db
            .prepare_cached("select lbl from lbls where tmpid=?1")?;
        query_n_collect_into_vec_string(cached_stmt.query((tid,)))?
    };

    let rel = {
        let mut cached_stmt = con
            .db
            .prepare_cached("select rel from files where tmpid=?1")?;
        let mut rows = cached_stmt.query((tid,))?;
        let row = rows.next()?.ok_or("TMPID unexpectedly invalid")?;
        let rel: String = row.get(0)?;
        rel
    };
    push_comps_to_pb(note_dir_clone, rel);
    note_dir_clone.push(INFO_FN);
    lbls.retain(|lbl| lbl.starts_with('_'));
    info_write(
        note_dir_clone,
        &InfoJsonElem {
            lbls,
            ..Default::default()
        },
    )
}

pub fn push_comps_to_pb(pb: &mut PathBuf, rel: String) {
    let comps = rel.split('/');
    for comp in comps {
        pb.push(comp);
    }
}

pub fn mk_pathstr_for_note(con: &super::Ctx, tid: i64) -> CustRes<String> {
    let rel = {
        let mut cached_stmt = con
            .db
            .prepare_cached("select rel from files where tmpid=?1")?;
        let mut rows = cached_stmt.query((tid,))?;
        let row = rows.next()?.ok_or("TMPID unexpectedly invalid")?;
        let rel: String = row.get(0)?;
        rel
    };
    let mut pb = con.def.ndir().clone();
    push_comps_to_pb(&mut pb, rel);
    pb.push("note");
    Ok(pb.into_os_string().into_string()?)
}

pub fn new_note(con: &mut super::Ctx, dirp: &mut path::PathBuf, fnms: Vec<&str>) -> CustRes<i64> {
    fs::create_dir_all(&dirp)?;
    dirp.push("note");
    fs::write(&dirp, b"\n")?;
    let mtime = systemtime2millis(fs::metadata(&dirp)?.modified()?);
    let mut rel = fnms.join("/");
    if !con.def.chosen_dir.is_empty() {
        rel.insert(0, '/');
        rel.insert_str(0, &con.def.chosen_dir);
    }
    let mut jobj = InfoJsonElem {
        att: Vec::default(),
        tmpid: 0,
        size: 1,
        rel,
        content: "\n".to_owned(),
        lbls: Vec::default(),
        mtime,
    };
    jobj.tmpid = ins_file(&con.db, jobj.mtime, &jobj.rel)?;
    debug_assert_after_eval!(dirp.pop());
    dirp.push(INFO_FN);
    info_write(dirp, &jobj)?;
    //con.def.chosen_note = jobj;
    Ok(jobj.tmpid)
}

pub fn ins_file(db: &rusqlite::Connection, mtime: i64, rel: &str) -> Result<i64, CustomErr> {
    let tmpid: i64 = super::get_avail_tmpid(db)?;
    let mut cached_stmt = db.prepare_cached("insert into files values(?1,?2,?3,?4,?5)")?;
    cached_stmt.execute((tmpid, 1, mtime, rel, "\n"))?;
    Ok(tmpid)
}

pub fn del_upper_dirs_if_useless(con: &super::Ctx, pb: &mut PathBuf) -> CustRes<()> {
    loop {
        pb.pop();
        //note actually there is no need to check len(). Just `pb == con.def.ndir()` should be fine. But still checking len() to be safe here. The reason is that deleting folders higher and higher are actually quite scary so let us do it with caution
        if pb.as_os_str().len() <= con.def.ndir().as_os_str().len() {
            return Ok(());
        }
        for dirent in pb.read_dir()? {
            let dirent = dirent?;
            let filty = dirent.file_type()?; //>will not traverse symlinks
            if filty.is_dir() {
                return Ok(());
            } else {
                warn!(
                    "{}{:?}",
                    "UNEXPECTED FILE TREATED AS TRASH: ",
                    dirent.path()
                );
            }
        }
        println!("{}{:?}", "FOLDER TO DELETE: ", pb);
        fs::remove_dir_all(&pb)?;
    }
}

pub fn get_all_folders(db: &rusqlite::Connection) -> CustRes<BTreeSet<String>> {
    let mut folders = BTreeSet::<String>::default();
    {
        let mut cached_stmt = db.prepare_cached("select rel from files")?;
        let mut rows = cached_stmt.query([])?;
        while let Some(row) = rows.next()? {
            let mut rel: String = row.get(0)?;
            let endidx = match rel.rfind('/') {
                None => {
                    continue;
                }
                Some(inner) => inner,
            };
            rel.truncate(endidx);
            folders.insert(rel);
        }
    }
    folders.insert("".to_owned());
    Ok(folders)
}

pub fn millis2display(ms: i64) -> String {
    use chrono::prelude::*;
    let ndt = NaiveDateTime::from_timestamp_millis(ms);
    let naive = match ndt {
        None => {
            return "FAILED TO CONV MS TO STR".to_owned();
        }
        Some(ndt_v) => ndt_v,
    };
    let datetime: DateTime<Utc> = DateTime::from_utc(naive, Utc);
    datetime.to_string()
}
