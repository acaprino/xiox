# Developers: vista flat e sotto-progetti — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Nella lista Projects di Developers un progetto dentro un altro sta sotto il padre, e una scelta `Layout: Tree / Flat` mostra tutti i progetti in una lista piatta con il percorso completo.

**Architecture:** Il motore (`xiox-core`, `dev/view.rs`) costruisce le righe già nella forma scelta; la pagina le disegna e manda un intent in più. La scelta è un campo di `Settings`, salvato in `pref.json` come le altre opzioni di Developers. Nessun componente nuovo: l'opzione è una `OptionView::Choice`, come `Deletion`. La pagina cambia in un solo punto (`rows.ts`): un sotto-progetto si vede sempre sotto il padre, dopo le cartelle da pulire.

**Tech Stack:** Rust (`xiox-model`, `xiox-app`, `xiox-core`), TypeScript (`apps/desktop/src`), test Rust su GitHub Actions, test della pagina con `node --test`.

**Spec:** `docs/superpowers/specs/2026-11-28-developers-flat-and-nested-design.md`

## Global Constraints

- **Niente `cargo`, `tsc`, `vite build`, `npm run build` su questa macchina.** Il Rust si scrive e non si esegue: lo prova il workflow Verify (`gh run watch`, `gh run view --log-failed`). `npm test` (solo `node --test`, non compila) si può eseguire. Nessuno può eseguire i test Rust qui: il testo dei test in questo piano è il contratto.
- Si lavora su `develop`, o su un ramo breve da `develop` con pull request in `develop` (anche quella fa girare Verify). Mai `main`, mai tag, mai cambiare la versione. Il primo commit porta anche la spec e questo piano.
- La pagina non scrive classi, stili o componenti propri. Prima di dire finito: `npx imago-check` e `npx --no sigrid-check apps/desktop/src`.
- Nessun cambiamento a cosa si può pulire: `toggle`, `select_all`, `selection`, il piano di pulizia e le conferme non cambiano comportamento.
- Stringhe dell'interfaccia in inglese. Esatte: opzione `Layout`, scelte `Tree` e `Flat`, id `layout`, valori `tree` e `flat`, chiave di dettaglio `with sub-projects`.
- Id delle righe invariati: `p:<id>` progetto, `a:<id>` cartella da pulire, `h:folder:<path>` intestazione di cartella.
- Vale solo per la categoria Projects.

## Come appare l'albero (deciso qui, da confermare dall'utente)

Un sotto-progetto è **sempre visibile** sotto il padre, anche col padre chiuso: una spunta messa dall'analisi non resta mai nascosta. La freccia del padre apre solo le sue cartelle da pulire, che vengono prima dei sotto-progetti.

```text
padre chiuso                                padre aperto
☐ > tri-llm-gateway        1.40 GB         ☐ v tri-llm-gateway        1.40 GB
☐   > admin-ui            310.0 MB         ☑     .venv                900.0 MB
☐   > vscode-extension     12.0 MB         ☐     .ruff_cache            2.0 MB
                                           ☐   > admin-ui             310.0 MB
                                           ☐   > vscode-extension      12.0 MB
```

## Review Focus

1. **Un progetto nella radice di un disco (`D:\`)** non diventa il padre di tutti i progetti del disco. (Task 1.)
2. **Percorsi scritti con maiuscole diverse** (`D:\Projects\gw` e `d:\projects\GW\admin-ui`): il padre si trova con `path_key`. (Task 1.)
3. **Prefisso che non è una cartella** (`app` e `app-old`): non sono padre e figlio. (Task 1.)
4. **Nipoti:** un progetto dentro un sotto-progetto sta sotto il sotto-progetto; il nonno li conta una volta sola. (Task 1.)
5. **Un `pref.json` o `settings.json` scritto prima di questo lavoro**, senza `layout`: si legge, e il layout è `Tree`. (Task 2.)

Limite noto, non coperto: un padre senza cartelle da pulire ma con sotto-progetti ha una freccia che non apre nulla.

---

### Task 1: Sotto-progetti sotto il padre (motore e pagina)

**Files:**
- Modify: `crates/xiox-core/src/dev/view.rs` (`project_rows`, `push_project`, `folder_tree`)
- Modify: `apps/desktop/src/rows.ts` (`sortBlock`, `visibleRows`)
- Test: `crates/xiox-core/src/dev/intent_tests.rs`, `apps/desktop/tests/rows.test.ts`

**Interfaces (private, in `view.rs`):**
- `fn nearest_parents(projects: &[ProjectReport]) -> Vec<Option<usize>>`: per ogni progetto, l'indice del progetto la cui `root` è l'antenato proprio più vicino della sua. Si cercano gli `ancestors()` della root (saltando la root stessa) in una mappa `path_key(root) -> indice`; mai un confronto per prefisso di stringa. Una root senza `parent()` (radice di un disco) non entra nella mappa.
- `fn folder_tree(projects: &[ProjectReport], parents: &[Option<usize>]) -> (Vec<usize>, Vec<Folder>)`: come oggi, ma entrano solo i progetti con `parents[i] == None`.
- `push_project` scrive la riga del progetto, le sue righe `a:`, poi (ricorsivo, nell'ordine di `scan.projects`) i sotto-progetti con `parent: Some("p:<id del padre>")`, `kind: RowKind::Item`.
- Righe in albero:
  - `name` del sotto-progetto: la sua root relativa a quella del padre (`admin-ui`, `tools\ext`);
  - `bytes` del padre: `reclaimable_bytes` suo più quello di tutti i discendenti;
  - `detail` del padre: `("size", <solo la sua>)` come oggi e, solo se ha sotto-progetti, in più `("with sub-projects", <totale, sys::fmt_size>)`;
  - `check` del padre: solo le sue cartelle (`tri_state`, come oggi).

**Interfaces (pagina, `rows.ts`):**
- `visibleRows`: una riga che non è `child` e ha per padre una riga non-`header` si vede quando si vede il padre, qualunque sia `expanded`. Le righe `child` seguono `expanded` come oggi.
- `sortBlock`: sotto una riga `item`, prima le righe `child` (ordinate tra loro), poi le altre (ordinate tra loro). Sotto un `header` nulla cambia.

- [ ] **Step 1: Test Rust che falliscono** in `intent_tests.rs`, dopo `projects_sit_in_a_tree_of_their_folders_whose_boxes_tick_the_free_ones_inside` (che deve restare verde senza modifiche):

```rust
/// `report` with another root: its one folder to clean keeps the id `tid(id)`.
fn rooted(id: &str, root: &str) -> ProjectReport {
    ProjectReport { root: PathBuf::from(root), ..report(id, ProjectStatus::Inactive, false, 10) }
}

/// gw holds ui and ext, ui holds deep; app-old only shares a prefix with app.
fn nested_scan() -> DevScan {
    DevScan {
        projects: vec![
            rooted("gw", r"D:\Projects\gw"),
            rooted("ui", r"d:\projects\GW\admin-ui"),
            rooted("ext", r"D:\Projects\gw\tools\ext"),
            rooted("deep", r"D:\Projects\gw\admin-ui\pkg\deep"),
            rooted("app", r"D:\Projects\app"),
            rooted("old", r"D:\Projects\app-old"),
        ],
        scanned_at_ms: xiox_app::now_ms(),
        ..Default::default()
    }
}

fn tree_of(v: &SectionView) -> Vec<(&str, Option<&str>)> {
    v.rows.iter().filter(|r| r.kind != RowKind::Child).map(|r| (r.id.as_str(), r.parent.as_deref())).collect()
}

#[test]
fn a_project_inside_another_sits_under_it_and_not_under_a_twin_folder() {
    let v = with_scan(nested_scan()).view(false);
    let top = r"h:folder:D:\Projects";
    assert_eq!(
        tree_of(&v),
        [(top, None), ("p:gw", Some(top)), ("p:ui", Some("p:gw")), ("p:deep", Some("p:ui")), ("p:ext", Some("p:gw")), ("p:app", Some(top)), ("p:old", Some(top))]
    );
    assert!(!v.rows.iter().any(|r| r.kind == RowKind::Header && r.id != top), "no twin folder heading");
    assert_eq!(row(&v, "p:ui").name, "admin-ui");
    assert_eq!(row(&v, "p:ext").name, r"tools\ext");
    assert_eq!(row(&v, "p:deep").name, r"pkg\deep");
    assert_eq!(row(&v, "p:gw").bytes, Some(40), "its own and every project inside it, once");
    assert_eq!(row(&v, "p:ui").bytes, Some(20));
    assert_eq!(row(&v, "p:app").bytes, Some(10));
    let with_subs = |id: &str| row(&v, id).detail.iter().any(|(k, _)| k == "with sub-projects");
    assert!(with_subs("p:gw") && with_subs("p:ui") && !with_subs("p:app") && !with_subs("p:deep"));
    assert_eq!(row(&v, &format!("a:{}", tid("ui"))).parent.as_deref(), Some("p:ui"));
}

#[test]
fn a_parent_box_ticks_only_its_own_folders_and_the_folder_box_ticks_the_sub_projects_too() {
    let mut st = with_scan(nested_scan());
    st.toggle("p:gw").unwrap();
    assert_eq!(st.sel_artifacts, ids(&["gw"]));
    assert_eq!(row(&st.view(false), "p:gw").check, Check::On, "its box is about its own folders");
    st.toggle("p:gw").unwrap();
    st.toggle(r"h:folder:D:\Projects").unwrap();
    assert_eq!(st.sel_artifacts, ids(&["gw", "ui", "ext", "deep", "app", "old"]));
    st.select_all(false).unwrap();
    assert!(st.sel_artifacts.is_empty());
}

#[test]
fn a_drive_root_project_is_never_a_parent() {
    let scan = DevScan { projects: vec![rooted("r", r"D:\"), rooted("a", r"D:\Projects\a")], scanned_at_ms: xiox_app::now_ms(), ..Default::default() };
    let v = with_scan(scan).view(false);
    assert_eq!(tree_of(&v), [("p:r", None), (r"h:folder:D:\Projects", None), ("p:a", Some(r"h:folder:D:\Projects"))]);
    assert_eq!(row(&v, "p:r").bytes, Some(10));
}
```

- [ ] **Step 2: Test della pagina che falliscono** in `apps/desktop/tests/rows.test.ts` (helper `row`, `child`, `header`, `ids` già nel file):

```ts
test('a project inside a project is always in sight, after the folders to clean', () => {
  const rows = [
    header('H'),
    row('P', 40, { parent: 'H' }), child('A', 'P', 5), row('S', 20, { parent: 'P' }), child('B', 'S', 9),
    row('Q', 30, { parent: 'H' }),
  ];
  assert.deepEqual([...rowDepths(rows)], [['H', 0], ['P', 1], ['A', 2], ['S', 2], ['B', 3], ['Q', 1]]);
  const sorted = sortRows(rows);
  assert.deepEqual(ids(sorted), ['H', 'P', 'A', 'S', 'B', 'Q'], 'the small folder to clean still comes before the larger sub-project');
  assert.deepEqual(ids(visibleRows(sorted, new Set())), ['H', 'P', 'S', 'Q']);
  assert.deepEqual(ids(visibleRows(sorted, new Set(['P']))), ['H', 'P', 'A', 'S', 'Q']);
  assert.deepEqual(ids(visibleRows(sorted, new Set(), new Set(['H']))), ['H'], 'a folded folder hides them all');
  assert.equal(headerBytes(rows, 'H'), 70, 'the sub-project is in its parent, not counted twice');
});
```

Run: `npm test` → FAIL su `sortRows` (S prima di A) e sul primo `visibleRows` (S nascosto).

- [ ] **Step 3: Implementa** motore e pagina secondo le Interfaces. `project_rows` calcola `parents` una volta.
- [ ] **Step 4:** Run `npm test` → PASS, tutta la suite (`children of a collapsed parent are not visible` deve restare verde).
- [ ] **Step 5: Commit** `feat(developers): a project inside another sits under it` (con spec e piano), push, `gh run watch`. Atteso: Verify verde, con i tre test Rust nuovi eseguiti. Se rosso: `gh run view --log-failed`, correggi, nuovo commit.

### Task 2: Scelta Layout e vista flat

**Files:**
- Modify: `crates/xiox-model/src/lib.rs` (`Settings`, nuovo enum)
- Modify: `crates/xiox-app/src/lib.rs:84`, `crates/xiox-app/src/tests.rs:683`, `crates/xiox-app/examples/smoke.rs:32` (i tre letterali `Settings { .. }`)
- Modify: `crates/xiox-core/src/view.rs` (`DevIntent`), `crates/xiox-core/src/app.rs:214`, `crates/xiox-core/src/dev/mod.rs`, `crates/xiox-core/src/dev/view.rs` (`options_view`, `project_rows`)
- Modify: `crates/xiox-core/src/dev/intent_tests.rs` (il `match` del replay, riga ~1126; test nuovi)
- Modify: `apps/desktop/fixtures/views.json` (le 10 viste di `views.developers`)
- Modify: `apps/desktop/src/types.ts`, `apps/desktop/src/intents.ts`
- Test: `apps/desktop/tests/intents.test.ts`, `crates/xiox-app/src/tests.rs`

**Interfaces:**
- Consumes: `nearest_parents`, `nested_scan`, `tree_of` del Task 1.
- Produces:
  - `xiox_model`: `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub enum ProjectLayout { #[default] Tree, Flat }` e il campo `#[serde(default)] pub layout: ProjectLayout` in fondo a `Settings`.
  - `xiox_core::view::DevIntent::SetLayout { layout: xiox_model::ProjectLayout }`, cioè JSON `{"kind":"setLayout","layout":"flat"}`. Nessun tipo `…View` in mezzo: l'enum ha già la forma del contratto.
  - `DevState::set_layout(&mut self, layout: ProjectLayout) -> Result<(), String>`: come `set_remove_mode` (`editable()?`, assegna, `save()`).
  - `options_view`: come **prima** opzione, `OptionView::Choice { id: "layout", label: "Layout", value: "tree" | "flat", choices: [("tree", "Tree"), ("flat", "Flat")], enabled }`.
  - Pagina: `DevIntent` in `types.ts` guadagna `| { kind: 'setLayout'; layout: 'tree' | 'flat' }`; `optionIntent` lo manda per `o.id === 'layout' && o.kind === 'choice'` con valore `tree` o `flat`.
- Righe in flat (`settings.layout == Flat`, solo `Section::Projects`):
  - nessuna riga `h:folder:`; ogni progetto, sotto-progetti compresi, ha `parent: None`, nell'ordine di `scan.projects`;
  - `name`: `r.name`; `sub`: la root completa (`r.root.display()`); il motivo resta in `detail` sotto `why`;
  - `bytes`: solo `reclaimable_bytes` del progetto; nessuna voce `with sub-projects`;
  - le righe `a:` restano sotto il loro progetto, identiche.

- [ ] **Step 1: Test Rust che falliscono.**

In `crates/xiox-core/src/dev/intent_tests.rs`:

```rust
#[test]
fn flat_lists_every_project_once_with_its_full_path_and_its_own_size() {
    let mut st = with_scan(nested_scan());
    st.set_layout(ProjectLayout::Flat).unwrap();
    let v = st.view(false);
    assert_eq!(tree_of(&v), [("p:gw", None), ("p:ui", None), ("p:ext", None), ("p:deep", None), ("p:app", None), ("p:old", None)]);
    assert_eq!(row(&v, "p:ui").name, "ui");
    assert_eq!(row(&v, "p:ui").sub.as_deref(), Some(r"d:\projects\GW\admin-ui"));
    assert_eq!(row(&v, "p:gw").bytes, Some(10), "its own size only: nothing is counted twice");
    assert!(!row(&v, "p:gw").detail.iter().any(|(k, _)| k == "with sub-projects"));
    assert_eq!(row(&v, &format!("a:{}", tid("ui"))).parent.as_deref(), Some("p:ui"));
    // The ticks do not depend on the layout.
    st.toggle("p:ui").unwrap();
    st.set_layout(ProjectLayout::Tree).unwrap();
    assert_eq!(st.sel_artifacts, ids(&["ui"]));
    assert_eq!(row(&st.view(false), "p:ui").parent.as_deref(), Some("p:gw"));
}

#[test]
fn the_layout_is_the_first_option_and_follows_the_setting() {
    let mut st = with_scan(nested_scan());
    let layout = |st: &DevState| match st.view(false).options.into_iter().next() {
        Some(OptionView::Choice { id, label, value, choices, enabled: true }) if id == "layout" && label == "Layout" => {
            (value, choices.into_iter().map(|c| (c.id, c.label)).collect::<Vec<_>>())
        }
        other => panic!("the first option is not Layout: {other:?}"),
    };
    let choices = vec![("tree".to_string(), "Tree".to_string()), ("flat".to_string(), "Flat".to_string())];
    assert_eq!(layout(&st), ("tree".to_string(), choices.clone()));
    st.set_layout(ProjectLayout::Flat).unwrap();
    assert_eq!(layout(&st), ("flat".to_string(), choices));
    // Like the other settings: not while a confirmation is open.
    st.toggle("p:gw").unwrap();
    st.open_confirm();
    assert_eq!(st.set_layout(ProjectLayout::Tree), st.set_remove_mode(RemoveMode::Permanent));
    assert!(st.set_layout(ProjectLayout::Tree).is_err());
}
```

(`ProjectLayout` e `RemoveMode` vanno aggiunti all'import `xiox_model::{…}` del file se non ci sono già.)

In `crates/xiox-core/src/view.rs`, dentro `intents_use_the_page_spelling`:

```rust
let intent: DevIntent = serde_json::from_value(json!({ "kind": "setLayout", "layout": "flat" })).unwrap();
assert_eq!(intent, DevIntent::SetLayout { layout: xiox_model::ProjectLayout::Flat });
assert!(serde_json::from_value::<DevIntent>(json!({ "kind": "setLayout", "layout": "grid" })).is_err());
```

In `crates/xiox-app/src/tests.rs`, dopo `the_old_settings_file_is_read_until_pref_json_has_its_section`:

```rust
/// Files written before the layout existed have no such field: they read, and the layout is the tree.
#[test]
fn settings_without_a_layout_read_as_the_tree_and_a_layout_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let store = prefs::Store::at(tmp.path().join("pref.json"));
    let legacy = tmp.path().join("settings.json");
    let mut old = serde_json::to_value(settings_walking(vec![])).unwrap();
    old["inactiveDays"] = 45.into();
    old.as_object_mut().unwrap().remove("layout").expect("Settings writes its layout");
    std::fs::write(&legacy, old.to_string()).unwrap();
    let back = load_settings_from(&store, &legacy);
    assert_eq!((back.inactive_days, back.layout), (45, ProjectLayout::Tree), "the old settings.json");
    store.write(prefs::DEVELOPERS, &old).unwrap();
    let back = load_settings_from(&store, &legacy);
    assert_eq!((back.inactive_days, back.layout), (45, ProjectLayout::Tree), "a pref.json section without the field");
    let mut flat = settings_walking(vec![]);
    flat.layout = ProjectLayout::Flat;
    store.write(prefs::DEVELOPERS, &flat).unwrap();
    assert_eq!(load_settings_from(&store, &legacy).layout, ProjectLayout::Flat);
}
```

- [ ] **Step 2: Test della pagina che fallisce** in `apps/desktop/tests/intents.test.ts`, accanto a quelli di `removeMode`:

```ts
const layout = choice('layout');
assert.deepEqual(optionIntent('developers', layout, 'flat'), { kind: 'setLayout', layout: 'flat' });
assert.deepEqual(optionIntent('developers', layout, 'tree'), { kind: 'setLayout', layout: 'tree' });
assert.equal(optionIntent('developers', layout, 'grid'), null);
```

Run: `npm test` → FAIL su queste righe.

- [ ] **Step 3: Implementa** secondo le Interfaces. Aggiungi `layout: ProjectLayout::Tree` ai tre letterali di `Settings`, e `DevIntent::SetLayout { layout } => st.set_layout(layout)` in `app.rs` e nel `match` del replay di `intent_tests.rs`.
- [ ] **Step 4: Fixture.** In `apps/desktop/fixtures/views.json`, in ognuna delle 10 viste di `views.developers`, aggiungi come primo elemento di `options`:
  `{"kind":"choice","id":"layout","label":"Layout","value":"tree","enabled":<lo stesso "enabled" dell'opzione removeMode di quella vista>,"choices":[{"id":"tree","label":"Tree"},{"id":"flat","label":"Flat"}]}`.
  Il test di deriva (`crates/xiox-core/src/drift.rs`) confronta le etichette delle opzioni con la vista vera: senza questo passo il Rust è rosso.
- [ ] **Step 5:** Run `npm test` → PASS (tutta la suite, comprese `fixture.test.ts`, `mockmodel.test.ts`, `mockrules.test.ts`).
- [ ] **Step 6: Commit** `feat(developers): Layout option, flat list of projects with full paths`, push, `gh run watch`. Atteso: Verify verde (test di `xiox-core`, `xiox-app`, `xiox-ui`; clippy `-D warnings`; pagina).

### Task 3: Guida, controlli del design e prova sulla finestra

**Files:**
- Modify: `docs/xiox-desktop.md` (capitolo Developers: tabella Options e paragrafo **Projects**)

- [ ] **Step 1: Guida.** Riga `Layout` nella tabella Options (`Tree`: i progetti nelle loro cartelle; `Flat`: una lista sola, il percorso completo sotto il nome, l'ordine vale su tutta la lista). Nel paragrafo **Projects**: un progetto dentro un altro sta sotto il padre ed è sempre in vista; il padre mostra la dimensione con i sotto-progetti; la sua casella spunta solo le sue cartelle; la freccia apre le cartelle da pulire.
- [ ] **Step 2:** Run `npx imago-check` e `npx --no sigrid-check apps/desktop/src` → entrambi senza errori.
- [ ] **Step 3: Commit** `docs(desktop): Layout and sub-projects in Developers`, push, Verify verde.
- [ ] **Step 4: Prova sulla finestra vera (la fa il proprietario).** `gh run download` del programma di debug del job Desktop, avviato **senza** `XIOX_UI_FAKE` (con quella variabile mostra le fixture, che ignorano Layout). In Developers, dopo Analyze: `tri-llm-gateway` una sola volta, con `admin-ui` e `vscode-extension` in vista sotto; con `Layout: Flat` l'ordine per dimensione su tutta la lista e il percorso completo sotto ogni nome; la scelta resta dopo il riavvio.

---

## Non in questo piano

- **La finestra finta (`XIOX_UI_FAKE=1`, `fake.rs`, `mockmodel.ts`) ignora `setLayout`**, come ignora `setRemoveMode`: mostra le righe delle fixture, che non hanno sotto-progetti. Le fixture guadagnano solo l'opzione.
- Campo di ricerca in Developers, avvisi grezzi in cima alla lista, velocità dell'analisi, worktree e copie non riconosciuti. Il secondo lavoro (worktree e copie) ha il suo disegno.
