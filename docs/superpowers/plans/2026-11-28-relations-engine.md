# Worktree e copie, piano 1: motore e riga di comando — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `xiox-project` sa trovare worktree, copie e cartelle col nome di **tutti** i repository del PC in una volta (indice dei repository + entrata nuova), con i tre stati dei worktree e il blocco delle copie che contengono worktree; la riga di comando usa le stesse regole.

**Architecture:** Un indice dei repository (`repo_index.rs`) calcola una volta l'identità di ogni repository (commit radice, `origin`) e li raggruppa. Un'entrata nuova (`relations.rs`, `analyze_relations`) fa per un repository solo worktree, copie e cartelle col nome, con la ricerca `ParentOnly` più le copie che l'indice le passa come `known`: produce un `ProjectScan` che `select`/`clean` accettano come oggi. Le regole di livello e blocco restano dentro `xiox-project`, per riga di comando e desktop.

**Tech Stack:** Rust (`xiox-git`, `xiox-project`, `xiox-cli`), `rayon = "1.12.0"` (già in `Cargo.lock`), CI GitHub Actions (Verify).

**Spec:** `docs/superpowers/specs/2026-11-28-developers-worktrees-copies-design.md` (sezioni 2, 3, 4, 5b, 7 e "I piani").

## Global Constraints

- **Niente `cargo`, `tsc`, `vite build`, `npm install` su questa macchina.** I test Rust si scrivono e non si eseguono: li prova Verify. Il testo dei test in questo piano è il contratto.
- Si lavora su un ramo breve `feat/relations-engine` da `develop`, con pull request in `develop`; Verify gira sulla pull request. Mai `main`, mai tag, mai la versione.
- Togliere un worktree resta `git worktree remove` (mai `--force`) più `prune`: **nessun ramo viene cancellato**.
- Livelli dei worktree (decisione del proprietario): HEAD contenuta in un ramo remoto o in un altro ramo locale → `recommended`; pulito con commit **solo** sul ramo del worktree → `caution`, non bloccato; HEAD staccata con commit su nessun ramo → bloccato `UnpushedCommits`. Modifiche, `locked`, sessione aperta bloccano come oggi; `danger` per attività nelle ultime 24 ore come oggi.
- Un'opzione di categoria (`xiox clean --worktrees`) prende solo voci `recommended`; il worktree `caution` lo prende solo `--include`.
- Una copia o cartella col nome che è un repository con worktree collegati che esistono è bloccata: `Block::HoldsWorktreeData`, motivo esatto `holds the git data of linked worktrees`, nome JSON `holds-worktree-data`.
- `classify` e i suoi test non cambiano: escludere i repository dalle cartelle col nome è un filtro della lista del desktop (piano 2), non di `xiox-project`.
- Testi in inglese, nella forma di `Block::reason`.
- Nessun test raggiunge il Cestino reale; le prove di pulizia sono `dry_run: true`.

## Review Focus

1. **Due copie che si somigliano solo per l'`origin` di un fork** (stesso `origin`, nessun commit radice in comune): stanno nello stesso gruppo, come oggi `classify` le chiama copie. (Task 4, test `same_origin_alone_makes_a_group`.)
2. **Un repository che git non deve leggere** (`.git` su un disco di rete o in una cartella cloud, `git_may_read` falso): non entra nell'indice e non fa chiamate a git. (Task 4.)
3. **Un worktree il cui ramo è lo stesso di un altro worktree o del principale non può esistere, ma un ramo con lo stesso nome remoto sì**: commit su `origin/feature` → `recommended`. (Task 2, già coperto da `head_kept_elsewhere`; test `a_worktree_whose_branch_is_pushed_is_recommended`.)
4. **Una copia trovata dall'indice lontano dal riferimento** (in un'altra cartella del disco): `clean` in prova (`dry_run`) la ritrova e non la salta come cambiata. (Task 5.)
5. **Lo stesso percorso in due scansioni** (copia di A e cartella col nome di B): resta una volta sola. (Task 6.)

---

### Task 0: Verify prova anche `xiox-git`, `xiox-project` e `xiox-cli`

Oggi Verify esegue `cargo test -p xiox-ui -p xiox-core -p xiox-app` (`.github/workflows/verify.yml:188`): i test dei crate di questo piano non girano mai.

**Files:**
- Modify: `.github/workflows/verify.yml:188` (e il commento sopra, riga 184)

- [ ] **Step 1:** Cambia il comando in `cargo test -p xiox-ui -p xiox-core -p xiox-app -p xiox-git -p xiox-project -p xiox-cli`; nel commento, una riga sul perché (i crate della riga di comando e del motore).
- [ ] **Step 2:** Commit `ci: Verify runs the tests of xiox-git, xiox-project and xiox-cli`, push del ramo, apri la pull request in `develop`, `gh run watch`.
- [ ] **Step 3: Leggi l'esito.** Verde: avanti. Rosso su test che questo piano non tocca: **fermati e riporta al proprietario** l'elenco dei test rossi (`gh run view --log-failed`): sono difetti che c'erano già, e decidere se correggerli, saltarli o togliere un crate dal comando è suo.

### Task 1: `xiox-git` conta i commit che stanno solo sul ramo del worktree

**Files:**
- Modify: `crates/xiox-git/src/project.rs` (accanto a `head_kept_elsewhere`, riga ~177; test nel modulo in fondo, accanto a `head_kept_elsewhere_true_for_detached_at_main_false_for_a_new_commit_on_detached`)

**Interfaces:**
- Produces: `pub fn own_commits(worktree: &Path) -> Option<u32>`: quanti commit raggiunge HEAD che nessun altro riferimento raggiunge (gli altri rami locali, tutti i rami remoti, i tag). Con un ramo corrente `b`: `git rev-list --count HEAD --not --exclude=refs/heads/<b> --branches --remotes --tags`; con HEAD staccata: `git rev-list --count HEAD --not --branches --remotes --tags`. `None` quando git non risponde o non c'è HEAD. Passa da `git(...)` del modulo (timeout, ambiente pulito).

- [ ] **Step 1: Test che falliscono**, con gli helper del modulo (`Tmp`, `init`, `commit_file`, `g`):

```rust
#[test]
fn own_commits_counts_what_only_the_worktree_branch_holds() {
    let t = Tmp::new();
    let main = t.sub("main");
    init(&main);
    commit_file(&main, "a.txt", "a");
    let wt = t.0.join("wt");
    g(&main, &["worktree", "add", "-q", "-b", "feature", &wt.to_string_lossy()]);
    assert_eq!(own_commits(&wt), Some(0), "a new branch at main's commit holds nothing of its own");
    commit_file(&wt, "b.txt", "b");
    commit_file(&wt, "c.txt", "c");
    assert_eq!(own_commits(&wt), Some(2));
    g(&main, &["branch", "keep", "feature"]);
    assert_eq!(own_commits(&wt), Some(0), "another local branch now holds them");
}

#[test]
fn own_commits_on_a_detached_head_counts_the_new_commit() {
    let t = Tmp::new();
    let main = t.sub("main");
    init(&main);
    commit_file(&main, "a.txt", "a");
    let det = t.0.join("det");
    g(&main, &["worktree", "add", "-q", "--detach", &det.to_string_lossy()]);
    assert_eq!(own_commits(&det), Some(0));
    commit_file(&det, "b.txt", "b");
    assert_eq!(own_commits(&det), Some(1));
}
```

- [ ] **Step 2:** Implementa `own_commits`.
- [ ] **Step 3:** Commit `feat(git): count the commits only a worktree's branch holds`.

### Task 2: Tre stati dei worktree in `xiox-project` e nella riga di comando

**Files:**
- Modify: `crates/xiox-project/src/lib.rs:142` (`Why::LinkedWorktree`)
- Modify: `crates/xiox-project/src/owned.rs:263-300` (`fn worktrees`)
- Modify: `crates/xiox-project/src/tests/owned.rs:200` (`worktree_with_a_commit_nobody_else_has_is_blocked_unpushed_commits`)
- Modify: `crates/xiox-cli/src/render.rs:102-104`, e ogni costruzione di `Why::LinkedWorktree` (`crates/xiox-cli/src/mcp.rs:1103`, `observe.rs:770`, `observe.rs:985`, `render.rs:670`, `testdata.rs:77`, `testdata.rs:88`, `testdata.rs:177`)
- Modify: `docs/xiox-cli.md` (paragrafo **`worktrees`**, righe ~130-138)
- Test: `crates/xiox-project/src/tests/owned.rs`, `crates/xiox-project/src/tests/clean.rs`

**Interfaces:**
- Consumes: `xiox_git::own_commits(&Path) -> Option<u32>` (Task 1); `WorktreeEntry.branch: Option<String>` (None = HEAD staccata).
- Produces: `Why::LinkedWorktree { branch: Option<String>, head: Option<String>, own_commits: Option<u32> }` (`own_commits`: i commit che solo il ramo del worktree tiene, quando il worktree è nel caso `caution`; `None` altrimenti). JSON `ownCommits`. `SCHEMA` resta 2 (campo aggiunto, nessuno tolto).

Regola nuova in `worktrees` (al posto della riga `head_kept_elsewhere != Some(true) → UnpushedCommits`, owned.rs:285):

| `head_kept_elsewhere` | `entry.branch` | effetto |
|---|---|---|
| `Some(true)` | qualsiasi | nessun blocco per i commit |
| `Some(false)` | `Some(_)` | nessun blocco; livello `Caution` anche senza altri blocchi; `own_commits = xiox_git::own_commits(&path)` |
| `Some(false)` | `None` | `Block::UnpushedCommits` (come oggi) |
| `None` | qualsiasi | `Block::UnpushedCommits` (come oggi: git non sa dire) |

- [ ] **Step 1: Test che falliscono** in `tests/owned.rs`. Rinomina e riscrivi `worktree_with_a_commit_nobody_else_has_is_blocked_unpushed_commits` in:

```rust
#[test]
fn a_worktree_whose_commits_are_only_on_its_branch_is_caution_and_not_blocked() {
    let (_t, base) = tmp();
    let root = repo(&base.join("proj"));
    let wt = worktree(&root, &base.join("proj-wt"), "feature");
    commit(&wt, "only-here.txt");
    let scan = analyze_project_without_sessions(&root, &opts(), &[base.clone()]).unwrap();
    let w = scan.items.iter().find(|i| i.category == Category::Worktrees).unwrap();
    assert!(w.blocked.is_empty(), "the branch stays: no commit is lost");
    assert_eq!(w.level, Level::Caution);
    assert!(matches!(&w.why, Why::LinkedWorktree { branch: Some(b), own_commits: Some(1), .. } if b == "feature"));
}

#[test]
fn a_detached_worktree_with_a_commit_on_no_branch_is_blocked_unpushed_commits() {
    // worktree --detach, poi un commit: blocked == [Block::UnpushedCommits], own_commits None
}

#[test]
fn a_worktree_whose_branch_is_pushed_is_recommended() {
    // clona proj in base/remote come `origin` di proj, worktree "feature" con un commit, push di feature:
    // blocked vuoto, level Recommended, own_commits None
}
```

(`opts()` = `Options { drives: false, history: false, ..Options::default() }`; se il file ha già un helper equivalente, usa quello.)

In `tests/clean.rs`:

```rust
#[test]
fn the_worktrees_flag_takes_a_recommended_worktree_but_not_a_caution_one() {
    // due worktree: uno a HEAD di main (recommended), uno con un commit sul suo ramo (caution)
    let chosen = select(&scan, &CleanRequest { categories: vec![Category::Worktrees], include: vec![], confirm_danger: vec![], mode: RemoveMode::Recycle }).unwrap();
    assert!(chosen.keys.contains(&recommended.key) && !chosen.keys.contains(&caution.key));
    let chosen = select(&scan, &CleanRequest { categories: vec![], include: vec![caution_path.clone()], confirm_danger: vec![], mode: RemoveMode::Recycle }).unwrap();
    assert!(chosen.keys.contains(&caution.key), "--include takes it");
}
```

- [ ] **Step 2:** Implementa la regola della tabella in `owned.rs`; aggiorna `Why::LinkedWorktree` e ogni sua costruzione con `own_commits: None` (nei dati di prova della riga di comando, dove il caso è `caution`, un valore reale).
- [ ] **Step 3:** `render.rs`: per `LinkedWorktree { branch: Some(b), own_commits: Some(n), .. }` la descrizione è `clean, {n} commit(s) only on branch {b} (the branch stays)` (`commit` al singolare per 1); gli altri casi come oggi. Test accanto a `render.rs:670`.
- [ ] **Step 4:** `docs/xiox-cli.md`, paragrafo `worktrees`: la tabella dei tre stati, e che togliere un worktree non cancella mai il suo ramo.
- [ ] **Step 5:** Commit `feat(project): a worktree whose commits are only on its branch is caution, not blocked`; push; Verify verde (test di `xiox-project`, `xiox-git`, `xiox-cli`).

### Task 3: Una copia con worktree suoi è bloccata

**Files:**
- Modify: `crates/xiox-project/src/lib.rs:74-113` (`Block`, `Block::reason`)
- Modify: `crates/xiox-project/src/search.rs` (`item_for`, riga ~341: dove i blocchi di `Copies` e `Named` si calcolano, righe ~345-352)
- Modify: `crates/xiox-project/src/tests/owned.rs:710` (`block_wording_has_one_source_of_truth`), `docs/xiox-cli.md` (tabella dei blocchi, riga ~255)
- Test: `crates/xiox-project/src/tests/search.rs`

**Interfaces:**
- Produces: `Block::HoldsWorktreeData`, `reason() == "holds the git data of linked worktrees"`, JSON `holds-worktree-data`.

- [ ] **Step 1: Test che falliscono** in `tests/search.rs`, accanto a `clone_elsewhere_is_a_copy_even_with_another_name`:

```rust
#[test]
fn a_copy_that_has_linked_worktrees_is_blocked() {
    // repo proj; clone in base/elsewhere/copy; `worktree(&copy, &base.join("copy-wt"), "x")`
    let item = /* la voce Copies di copy */;
    assert!(item.blocked.contains(&Block::HoldsWorktreeData));
    assert_eq!(Block::HoldsWorktreeData.reason(), "holds the git data of linked worktrees");
}

#[test]
fn a_copy_whose_only_worktree_was_deleted_is_not_blocked_for_it() {
    // come sopra, poi la cartella copy-wt cancellata (prunable): nessun HoldsWorktreeData
}
```

E aggiungi `Block::HoldsWorktreeData` all'elenco di `block_wording_has_one_source_of_truth`.

- [ ] **Step 2:** Implementa: in `item_for`, per `Category::Copies` e `Category::Named`, quando la cartella è un repository (`is_git_repo`) e `xiox_git::list_worktrees(dir)` ha una voce `!is_main && !prunable`, aggiungi il blocco.
- [ ] **Step 3:** Commit `feat(project): a copy that holds the git data of linked worktrees is blocked`; push; Verify verde.

### Task 4: L'indice dei repository

**Files:**
- Create: `crates/xiox-project/src/repo_index.rs`
- Modify: `crates/xiox-project/src/lib.rs` (`mod repo_index; pub use repo_index::{RepoIdentity, RepoIndex, repositories_under};`), `crates/xiox-project/Cargo.toml` (`rayon = "1.12.0"`, la stessa di `xiox-scanner`)
- Test: `crates/xiox-project/src/tests/repo_index.rs` (nuovo, registrato in `tests/mod.rs`)

**Interfaces:**
- Produces:
  - `pub struct RepoIdentity { pub root: PathBuf, pub root_commits: Vec<String>, pub origin: Option<String> }` (`origin` già passato da `xiox_git::normalize_remote`).
  - `pub struct RepoIndex { pub repos: Vec<RepoIdentity>, groups: Vec<Vec<usize>> }`
  - `impl RepoIndex { pub fn build(roots: &[PathBuf]) -> RepoIndex; pub fn group_of(&self, root: &Path) -> Option<&[usize]>; pub fn reference(&self, group: &[usize], activity: &dyn Fn(&Path) -> Option<i64>) -> usize }`
  - `pub(crate) fn groups(ids: &[(Vec<String>, Option<String>)]) -> Vec<Vec<usize>>` (pura)
  - `pub fn repositories_under(roots: &[(PathBuf, usize)]) -> Vec<PathBuf>`

Regole:
- `build`: toglie i doppioni per `path_key`; tiene solo le cartelle che sono un repository (`search::is_git_repo`) e che git può leggere (`gitref::git_may_read(dir, git_home().as_deref())`); per ognuna `xiox_git::root_commits` e `xiox_git::origin_url` in parallelo (`rayon`).
- `groups`: due repository stanno insieme se hanno un commit radice in comune **o** lo stesso `origin` normalizzato; la relazione è transitiva (unione e ricerca). Solo i gruppi con almeno 2 membri; i membri in ordine di `path_key` della radice, i gruppi in ordine del loro primo membro.
- `group_of`: il gruppo che contiene quella radice (confronto con `path_key`), `None` se è da sola.
- `reference`: il membro con l'attività più recente secondo `activity`; a pari merito, o senza attività, il primo per `path_key`.
- `repositories_under`: cammino limitato come `search::walk` (nessun link seguito, `is_system_dir`, `AppData` escluso, non scende dentro un repository trovato), fino alla profondità data per radice; restituisce le radici dei repository, ordinate per `path_key`, senza doppioni.

- [ ] **Step 1: Test che falliscono** (tabella su `groups`, poi repository veri con gli helper di `tests/fixtures.rs`):

```rust
#[test]
fn groups_join_by_root_commit_or_origin_transitively() {
    let id = |roots: &[&str], origin: Option<&str>| (roots.iter().map(|s| s.to_string()).collect::<Vec<_>>(), origin.map(str::to_string));
    let ids = vec![
        id(&["r1"], None),                     // 0: A
        id(&["r1", "r2"], Some("gh/x")),       // 1: B, legato ad A per r1
        id(&["r9"], Some("gh/x")),             // 2: C, legato a B per l'origin
        id(&["r5"], None),                     // 3: D, da solo
    ];
    assert_eq!(groups(&ids), vec![vec![0, 1, 2]]);
}

#[test]
fn same_origin_alone_makes_a_group() {
    let ids = vec![(vec!["a".into()], Some("gh/fork".into())), (vec!["b".into()], Some("gh/fork".into()))];
    assert_eq!(groups(&ids), vec![vec![0, 1]]);
}

#[test]
fn build_groups_a_clone_and_leaves_an_unrelated_repository_alone() {
    // repo proj, clone(proj → elsewhere/zzz), repo other: un gruppo {proj, zzz}; other: group_of == None
}

#[test]
fn reference_is_the_most_recent_and_ties_go_to_the_first_path() {
    // gruppo di due: activity = Some(1) e Some(2) → il secondo; activity sempre None → il primo per path_key
}

#[test]
fn repositories_under_stops_at_a_repository_and_at_the_depth() {
    // base/a (repo), base/a/inner (repo dentro: non elencato), base/x/y/z/deep (repo a profondità 4 con depth 3: non elencato)
    assert_eq!(repositories_under(&[(base.clone(), 3)]), vec![canon(&base.join("a"))]);
}
```

(Un test per `git_may_read`: un repository il cui `.git` è un file che punta fuori dai dischi locali non entra in `build`; usa l'helper che `tests/gitref.rs` usa già per questo caso.)

- [ ] **Step 2:** Implementa `repo_index.rs`.
- [ ] **Step 3:** Commit `feat(project): an index of the PC's repositories, grouped by identity`; push; Verify verde.

### Task 5: L'entrata nuova `analyze_relations`

**Files:**
- Create: `crates/xiox-project/src/relations.rs`
- Modify: `crates/xiox-project/src/lib.rs` (`mod relations; pub use relations::{analyze_relations, RelationsInput};`), `crates/xiox-project/src/owned.rs` (`fn worktrees` diventa `pub(crate)` se non lo è)
- Test: `crates/xiox-project/src/tests/relations.rs` (nuovo)

**Interfaces:**
- Consumes: `owned::worktrees`, `search::search`, `history::assess`, `resolve::resolve_full`, le regole dei Task 2-3.
- Produces:
  - `pub struct RelationsInput<'a> { pub sessions: &'a [xiox_app::Preloaded], pub sessions_failed: bool, pub now_ms: i64 }`
  - `pub fn analyze_relations(path: &Path, known: &[PathBuf], input: &RelationsInput) -> Result<ProjectScan, Error>`: un `ProjectScan` con **solo** voci `Worktrees`, `Copies`, `Named`; `search == SearchMode::ParentOnly`; `history == None`; `branches` vuoto; le note di `resolve_full`, `PrunableWorktree` e `SessionsNotRead` quando `sessions_failed`. Opzioni interne: `Options { drives: false, measure: true, admin: false, history: false }`. Nessun `engine_reports` (i blocchi per sessione aperta vengono da `live_session_in` sulle `sessions`).

- [ ] **Step 1: Test che falliscono**, con repository veri in cartelle temporanee e `sessions: &[]`:

```rust
#[test]
fn relations_list_worktrees_copies_and_named_folders_and_nothing_else() {
    // base/proj (repo, con un target/ di build), worktree in base/far/proj-wt, clone in base/elsewhere/copy,
    // cartella non-git base/proj-old (sorella di proj)
    let scan = analyze_relations(&proj, &[copy.clone()], &input).unwrap();
    let cats: HashSet<Category> = scan.items.iter().map(|i| i.category).collect();
    assert_eq!(cats, HashSet::from([Category::Worktrees, Category::Copies, Category::Named]));
    assert!(scan.items.iter().any(|i| i.category == Category::Worktrees && i.path.as_deref() == Some(&*wt)), "a worktree outside the folder");
    assert!(scan.items.iter().any(|i| i.category == Category::Copies && i.path.as_deref() == Some(&*copy)), "a copy far away, given as known");
    assert!(scan.items.iter().any(|i| i.category == Category::Named && i.path.as_deref() == Some(&*old)));
    assert_eq!(scan.search, SearchMode::ParentOnly);
    assert!(scan.history.is_none());
}

#[test]
fn a_relations_scan_round_trips_through_select_and_clean_dry_run() {
    let scan = analyze_relations(&proj, &[copy.clone()], &input).unwrap();
    let chosen = select(&scan, &CleanRequest { categories: vec![], include: vec![copy.clone()], confirm_danger: vec![], mode: RemoveMode::Recycle }).unwrap();
    let out = clean_without_sessions(&scan, &chosen, RemoveMode::Recycle, true, &[]);
    let item = out.items.iter().find(|o| o.path.as_deref() == Some(&*copy)).unwrap();
    assert!(!matches!(item.outcome, Outcome::Skipped { .. }), "the copy is found again, not skipped as changed: {:?}", item.outcome);
    assert!(copy.exists(), "a dry run removes nothing");
}
```

(La copia di prova è pulita e raggiunta dal progetto, quindi `caution` e non bloccata; usa l'`env` a orologio spostato di `tests/fixtures.rs` per non cadere in `danger`.) **Se il secondo test fallisce con `Skipped`, fermati e riporta**: vuol dire che `clean` non accetta una scansione di relazioni, e il piano va rivisto (serve un'entrata di riconvalida).

- [ ] **Step 2:** Implementa `relations.rs`.
- [ ] **Step 3:** Commit `feat(project): analyze_relations, the worktrees, copies and named folders of one repository`; push; Verify verde.

### Task 6: Ogni percorso una volta sola

**Files:**
- Modify: `crates/xiox-project/src/relations.rs`
- Test: `crates/xiox-project/src/tests/relations.rs`

**Interfaces:**
- Produces: `pub fn keep_once(scans: &mut [ProjectScan], references: &HashSet<String>)`, dove `references` sono le `path_key` delle radici dei riferimenti dei gruppi. Regole, nell'ordine:
  1. una voce `Copies` o `Named` il cui percorso è la radice di un riferimento si toglie;
  2. tra le voci `Copies`/`Named` con lo stesso percorso in scansioni diverse resta una: `Copies` prima di `Named`; tra due `Copies`, quella della scansione il cui progetto è un riferimento; tra due `Named`, quella del progetto col nome più lungo; a pari merito la prima scansione;
  3. le voci `Worktrees` con lo stesso percorso restano una (la prima).

- [ ] **Step 1: Test che fallisce**, guidato dai dati, con `ProjectScan` costruite a mano (un helper `scan(root, name, items)` e `item(cat, path)` nel file di test; nessun git):

| scansione | progetto | voci prima | voci dopo |
|---|---|---|---|
| A (riferimento) | `D:\p\supads` | Copies `D:\x\supads-copy`, Named `D:\p\supads-work` | le stesse |
| C | `D:\x\supads-copy` | Copies `D:\p\supads` | nessuna (regola 1: è la radice del riferimento) |
| B | `D:\p\supads-studio` | Named `D:\p\supads-studio-old` | la stessa |
| D | `D:\q\supads` (non riferimento) | Copies `D:\x\supads-copy` | nessuna (regola 2: la tiene A, riferimento) |

```rust
#[test]
fn keep_once_leaves_each_folder_in_one_scan() {
    // le quattro scansioni della tabella; references = {path_key(D:\p\supads)}
    keep_once(&mut scans, &references);
    assert_eq!(paths(&scans[0]), [r"D:\x\supads-copy", r"D:\p\supads-work"]);
    assert!(paths(&scans[1]).is_empty());
    assert_eq!(paths(&scans[2]), [r"D:\p\supads-studio-old"]);
    assert!(paths(&scans[3]).is_empty());
}

#[test]
fn a_folder_named_after_two_projects_goes_to_the_longer_name() {
    // "supads" e "supads-work" (entrambi progetti): D:\p\supads-work-build è Named in tutti e due → resta in supads-work
}
```

- [ ] **Step 2:** Implementa `keep_once`.
- [ ] **Step 3:** Commit `feat(project): keep_once, one listing per folder across the scans`; push; Verify verde; la pull request `feat/relations-engine` → `develop` è pronta per la revisione finale e per il proprietario.

---

## I piani 2-4 (un paragrafo ciascuno; si scrivono dopo il piano 1)

**Piano 2: `Copies` nel desktop.** `xiox-core` dipende da `xiox-project`. `xiox-app::run_scan` conserva in `DevScan` le sessioni lette (`Vec<Preloaded>` e se il loro elenco è fallito) e le radici dei repository trovate dall'indice del disco (`find_dirs_named(&[".git"], ..)` prima del `drop(index)`, riga 499); senza indice, `xiox_project::repositories_under` sulle radici che le impostazioni permettono (`all_drives`, `--only-root`). Dopo `run_scan`, `real_scan` costruisce `RepoIndex`, sceglie i riferimenti (attività dai `ProjectReport`), chiama `analyze_relations` in parallelo per ogni repository, applica `keep_once` e il filtro "un repository non è una cartella col nome", con avanzamento "Looking for worktrees and copies… n of m repositories" e un tetto di tempo da decidere dopo la misura. Nuova `Section::Copies` / `DevCategory::Copies` (tutti i `match` elencati dal rapporto: `dev/mod.rs` :44, :54, :649, :937, :1042, :1074, :1092, :1101, :1110, :1359; `dev/view.rs` :98, :182, :193, :598; la lista di `nav_view`; `others_note`), `LevelView::Danger` con `eligible` che esclude anche `Danger`, conferma con una casella per ogni voce `danger` (via `Chosen::confirm`), pulizia con `select` + `clean` per scansione, `scanned_at_ms` della scansione per la regola dei 30 minuti; pagina: categoria `copies` (intents, types, mockmodel, fake.rs, fixture). Nessun test raggiunge il Cestino reale.

**Piano 3: `Worktrees` del desktop.** Le righe di `Worktrees` vengono dalle voci `Worktrees` delle scansioni di relazioni (al posto di `scan.worktrees` + `other_worktrees`); le cartelle di build di un worktree restano figlie della sua riga e si puliscono dal motore come oggi (l'allow-list di `checked_plan` accetta anche gli artefatti dei worktree non gestiti da agenti); partono spuntati solo i worktree `recommended`; la rimozione intera passa da `xiox-project`. È il piano che può peggiorare il comportamento di oggi: prova sulla finestra vera prima di unirlo.

**Piano 4: "Refresh this project".** Un intent sulla riga selezionata di `Projects` (dettagli e palette): rilegge le sessioni, rifà `analyze_relations` per quel repository con i membri del suo gruppo dall'indice conservato, sostituisce le sue righe in `Worktrees` e `Copies`, con la propria ora per la regola dei 30 minuti.

**Prima del piano 2:** una misura in sola lettura sul PC del proprietario, **solo con il suo permesso** (`AGENTS.md`): quanti repository, quanti worktree collegati, quanti millisecondi per una chiamata a git. Decide se lo stato dei worktree si calcola in Analyze o solo quando si apre `Worktrees`.
