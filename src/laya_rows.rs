//! `--laya-rows IN OUT`: training and test rows for fine-tuning the Laya model, written the
//! way `laya::guess` asks it, so what it learns is what it is asked.
//!
//! `IN` is either a folder of synthetic data (`training/laya-uni/synthetic`, with a
//! `programs.json` per group) or a folder of real files laid out as
//! `[faculty/]program/module/…` (open course material, or a sorted study folder).
//! Each row: `{state, questions: {module: {type, instructions, criteria}}, expected | gold, meta}`,
//! the `{state, questions, expected}` format `laya.train.finetune` reads.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::laya;

/// At most this many options per question (the other modules of the program, at random).
const MAX_OPTIONS: usize = 12;
/// Rows made from each synthetic record (each with other options and option wording).
const ROWS_PER_RECORD: usize = 2;

#[derive(Deserialize, Clone)]
struct Program {
    program: String,
    #[serde(default)]
    faculty: String,
    #[serde(default)]
    lang: String,
    modules: Vec<Module>,
}

#[derive(Deserialize, Clone)]
struct Module {
    name: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
}

#[derive(Deserialize)]
struct Record {
    program: String,
    module: Option<String>,
    #[serde(default)]
    lang: String,
    #[serde(default)]
    kind: String,
    file_name: String,
    text: String,
}

/// A small seeded random generator (xorshift), so the same input gives the same rows.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

/// One example: a file, the program it belongs to, and its module (None: nothing fits).
struct Example {
    program: usize,
    module: Option<usize>,
    name: String,
    text: String,
    meta: Value,
}

pub fn run(input: &Path, output: &Path) -> Result<usize, String> {
    let (programs, examples) =
        if find_programs(input).is_empty() { from_folders(input)? } else { from_synthetic(input)? };
    let rows_each = if examples.iter().any(|e| e.meta["source"] == "synthetic") { ROWS_PER_RECORD } else { 1 };
    let mut rng = Rng(0x5eed_1a7a_u64);
    let mut out = std::io::BufWriter::new(fs::File::create(output).map_err(|e| e.to_string())?);
    let mut n = 0;
    for example in &examples {
        for _ in 0..rows_each {
            let row = row(&programs, example, &mut rng);
            writeln!(out, "{row}").map_err(|e| e.to_string())?;
            n += 1;
        }
    }
    Ok(n)
}

fn find_programs(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("programs.json"))
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

fn from_synthetic(dir: &Path) -> Result<(Vec<Program>, Vec<Example>), String> {
    let folder_name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // All programs first: a group's records may belong to programs of another group
    // (the mixed-language files). Each program remembers its group, which names the faculty.
    let mut programs: Vec<Program> = Vec::new();
    let mut group_of: Vec<String> = Vec::new();
    for file in find_programs(dir) {
        let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let list: Vec<Program> = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        group_of.extend(list.iter().map(|_| folder_name(file.parent().unwrap_or(dir))));
        programs.extend(list);
    }
    let mut examples = Vec::new();
    for file in find_programs(dir) {
        let group = file.parent().unwrap_or(dir);
        let mut record_files: Vec<PathBuf> = fs::read_dir(group)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("records-")))
            .collect();
        record_files.sort();
        for records in record_files {
            for (i, line) in fs::read_to_string(&records).map_err(|e| e.to_string())?.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let where_ = format!("{}:{}", records.display(), i + 1);
                let r: Record = serde_json::from_str(line).map_err(|e| format!("{where_}: {e}"))?;
                let program = programs
                    .iter()
                    .position(|p| p.program == r.program)
                    .ok_or(format!("{where_}: unknown program {}", r.program))?;
                let module = match &r.module {
                    None => None,
                    Some(m) => Some(
                        programs[program]
                            .modules
                            .iter()
                            .position(|x| x.name == *m)
                            .ok_or(format!("{where_}: unknown module {m}"))?,
                    ),
                };
                let p = &programs[program];
                let meta = json!({"source": "synthetic", "faculty": group_of[program], "field": p.faculty,
                    "program": p.program, "lang": if r.lang.is_empty() { &p.lang } else { &r.lang },
                    "kind": r.kind, "mixed": folder_name(group) != group_of[program], "file": where_});
                examples.push(Example { program, module, name: r.file_name, text: r.text, meta });
            }
        }
    }
    Ok((programs, examples))
}

/// `[faculty/]program/module/…files…`: a folder with modules directly under program folders.
/// The faculty level is optional; files deeper in a module (subfolders) count for it.
fn from_folders(root: &Path) -> Result<(Vec<Program>, Vec<Example>), String> {
    let dirs = |p: &Path| -> Vec<PathBuf> {
        let mut d: Vec<PathBuf> = fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).collect();
        d.retain(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')));
        d.sort();
        d
    };
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // With a faculty level, every folder two levels down (a program) has folders (modules) in it.
    // shortcut: a study folder whose modules all have subfolders looks like that too; add a flag if needed.
    let has_faculty = dirs(root).iter().all(|f| {
        let programs = dirs(f);
        !programs.is_empty() && programs.iter().all(|p| !dirs(p).is_empty())
    });
    let program_dirs: Vec<(String, PathBuf)> = if has_faculty {
        dirs(root).iter().flat_map(|f| dirs(f).into_iter().map(move |p| (name(f), p))).collect()
    } else {
        dirs(root).into_iter().map(|p| (String::new(), p)).collect()
    };
    let mut programs = Vec::new();
    let mut examples = Vec::new();
    for (faculty, dir) in program_dirs {
        let module_dirs = dirs(&dir);
        let program = programs.len();
        programs.push(Program {
            program: name(&dir),
            faculty: faculty.clone(),
            lang: String::new(),
            modules: module_dirs.iter().map(|m| Module { name: name(m), code: None, keywords: Vec::new() }).collect(),
        });
        for (module, mdir) in module_dirs.iter().enumerate() {
            for file in files_in(mdir) {
                let text = crate::extract::extract_text(&file);
                let meta = json!({"source": "folder", "faculty": faculty, "program": name(&dir),
                    "file": file.strip_prefix(root).unwrap_or(&file).to_string_lossy()});
                examples.push(Example { program, module: Some(module), name: name(&file), text, meta });
            }
        }
    }
    if examples.is_empty() {
        return Err(format!("no files in {} (expected [faculty/]program/module/files)", root.display()));
    }
    Ok((programs, examples))
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        if path.is_dir() {
            out.extend(files_in(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// The question for one example: its program's modules (some of them when there are many,
/// sometimes with a few from other programs), worded the different ways people name modules.
fn row(programs: &[Program], ex: &Example, rng: &mut Rng) -> String {
    let program = &programs[ex.program];
    let mut others: Vec<usize> = (0..program.modules.len()).filter(|i| Some(*i) != ex.module).collect();
    rng.shuffle(&mut others);
    let room = MAX_OPTIONS - usize::from(ex.module.is_some());
    others.truncate(room.min(others.len()).max(2.min(others.len())));
    // A minor or a second program: a few modules from elsewhere.
    let mut foreign: Vec<(usize, usize)> = Vec::new();
    if programs.len() > 1 && rng.chance(30) {
        for _ in 0..1 + rng.below(3) {
            let p = rng.below(programs.len());
            if p != ex.program && !programs[p].modules.is_empty() {
                foreign.push((p, rng.below(programs[p].modules.len())));
            }
        }
    }
    let with_keywords = rng.chance(50);
    let with_program = rng.chance(30);
    let with_code = rng.chance(20);
    let label = |p: usize, m: &Module| {
        let mut name = m.name.clone();
        if with_code && let Some(code) = &m.code {
            name = format!("{code} {name}");
        }
        if with_program { format!("{} › {name}", programs[p].program) } else { name }
    };
    let mut options: Vec<(String, Vec<String>)> = others
        .iter()
        .map(|i| (ex.program, &program.modules[*i]))
        .chain(foreign.iter().map(|(p, m)| (*p, &programs[*p].modules[*m])))
        .chain(ex.module.map(|m| (ex.program, &program.modules[m])))
        .map(|(p, m)| (label(p, m), if with_keywords { m.keywords.clone() } else { Vec::new() }))
        .collect();
    let answer = ex.module.map(|m| label(ex.program, &program.modules[m]));
    options.sort_by(|a, b| a.0.cmp(&b.0));
    options.dedup_by(|a, b| a.0 == b.0);
    rng.shuffle(&mut options);
    let criteria = ordered_object(
        options.iter().map(|(l, k)| (l.clone(), if k.is_empty() { Value::Null } else { Value::String(k.join(", ")) })),
    );
    let question = json!({"type": "choice", "instructions": laya::INSTRUCTIONS, "criteria": "@CRITERIA@"});
    let mut row = json!({"state": laya::state_for(&ex.name, &ex.text), "questions": {"module": question},
        "meta": ex.meta});
    match answer {
        Some(a) => row["expected"] = json!({"module": a}),
        None => {
            // Nothing fits: every option equally (un)likely.
            let p = 1.0 / options.len() as f64;
            let probabilities = ordered_object(options.iter().map(|(l, _)| (l.clone(), json!(p))));
            row["gold"] = json!({"module": {"probabilities": "@PROBS@"}});
            return splice(splice(row.to_string(), "@CRITERIA@", &criteria), "@PROBS@", &probabilities);
        }
    }
    splice(row.to_string(), "@CRITERIA@", &criteria)
}

/// A JSON object in this order (serde_json's own maps sort their keys, which would
/// put the options in alphabetical order).
fn ordered_object(pairs: impl Iterator<Item = (String, Value)>) -> String {
    let body: Vec<String> = pairs.map(|(k, v)| format!("{}:{v}", Value::String(k))).collect();
    format!("{{{}}}", body.join(","))
}

/// Put a ready-made JSON text where a placeholder string is.
fn splice(json: String, placeholder: &str, value: &str) -> String {
    json.replacen(&format!("\"{placeholder}\""), value, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn synthetic_records_become_questions_like_the_app_asks() {
        let tmp = tempfile::tempdir().unwrap();
        let group = tmp.path().join("law");
        write(
            &group.join("programs.json"),
            r#"[{"program": "Jus", "faculty": "law", "lang": "de", "modules": [
                {"name": "Strafrecht AT", "code": "JUS1", "keywords": ["StGB", "Vorsatz"]},
                {"name": "Strafrecht BT"}, {"name": "Privatrecht I"}]}]"#,
        );
        write(
            &group.join("records-jus.jsonl"),
            "{\"program\": \"Jus\", \"module\": \"Strafrecht AT\", \"lang\": \"de\", \"kind\": \"case\", \
              \"file_name\": \"Fall3.pdf\", \"text\": \"Irrtum über Tatumstände\"}\n\
             {\"program\": \"Jus\", \"module\": null, \"kind\": \"admin\", \"file_name\": \"Mensa.pdf\", \"text\": \"Menü\"}\n",
        );
        // Files of another group's program (the mixed-language ones) count for that program.
        let mixed = tmp.path().join("a-mixed");
        write(&mixed.join("programs.json"), "[]");
        write(
            &mixed.join("records-mixed.jsonl"),
            "{\"program\": \"Jus\", \"module\": \"Strafrecht BT\", \"lang\": \"en\", \"file_name\": \"paper.pdf\", \"text\": \"Theft\"}\n",
        );
        let out = tmp.path().join("rows.jsonl");
        assert_eq!(run(tmp.path(), &out).unwrap(), 6);
        let rows: Vec<Value> =
            fs::read_to_string(&out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!((&rows[0]["meta"]["faculty"], &rows[0]["meta"]["mixed"]), (&json!("law"), &json!(true)));
        let rows = &rows[2..];
        let first = &rows[0];
        assert_eq!(first["state"], laya::state_for("Fall3.pdf", "Irrtum über Tatumstände"));
        let q = &first["questions"]["module"];
        assert_eq!(q["instructions"], laya::INSTRUCTIONS);
        let expected = first["expected"]["module"].as_str().unwrap();
        assert!(expected.ends_with("Strafrecht AT"), "{expected}");
        let criteria = q["criteria"].as_object().unwrap();
        assert_eq!(criteria.len(), 3);
        // Laya renders `label: keywords`, which is what the app's option text is.
        for (label, kw) in criteria {
            let keywords: Vec<String> =
                kw.as_str().map(|k| k.split(", ").map(String::from).collect()).unwrap_or_default();
            let rendered = if kw.is_null() { label.clone() } else { format!("{label}: {}", kw.as_str().unwrap()) };
            assert_eq!(laya::option(label, &keywords), rendered);
        }
        // Nothing fits: uniform soft target instead of an answer.
        let none = &rows[2];
        assert!(none.get("expected").is_none());
        let probs = none["gold"]["module"]["probabilities"].as_object().unwrap();
        assert!(probs.values().all(|p| (p.as_f64().unwrap() - 1.0 / probs.len() as f64).abs() < 1e-9));
    }

    #[test]
    fn folders_of_modules_become_rows_with_the_folder_as_answer() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("Medizin/Anatomie/VL1.txt"), "Obere Extremität");
        write(&tmp.path().join("Medizin/Physiologie/Sub/Blatt.txt"), "Herzzyklus");
        let out_dir = tempfile::tempdir().unwrap();
        let out = out_dir.path().join("rows.jsonl");
        assert_eq!(run(tmp.path(), &out).unwrap(), 2);
        let rows: Vec<Value> =
            fs::read_to_string(&out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert!(rows[1]["expected"]["module"].as_str().unwrap().ends_with("Physiologie"));
        assert_eq!(rows[1]["meta"]["program"], "Medizin");
        assert!(rows[1]["state"].as_str().unwrap().starts_with("File name: Blatt.txt"));
    }
}
