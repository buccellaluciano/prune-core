//! Las otras secciones de la interfaz interactiva: apps de Windows e historial.
//!
//! Es el mismo `App` de `tui.rs` (este archivo es un módulo hijo, por eso ve sus campos):
//! acá están las teclas y el dibujo de las secciones que no son la de ajustes.

use super::*;
use prune_engine::appx::RemoveResult;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Tweaks,
    Bloat,
    History,
}

fn read_only() -> String {
    tr("Solo lectura: P-Rune está instalado para todo el equipo. Abrí una terminal de administrador para cambiar algo.").into()
}

/// Una lista a pantalla completa con su fila seleccionada (o un texto si está vacía).
fn render_list(f: &mut Frame, area: Rect, title: &str, rows: Vec<ListItem<'static>>, state: &mut ListState, selected: usize, empty: &str) {
    let is_empty = rows.is_empty();
    let list = List::new(rows)
        .block(Block::bordered().border_style(fg(Color::Cyan)).title(format!(" {title} ")))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol(" ");
    state.select((!is_empty).then_some(selected));
    f.render_stateful_widget(list, area, state);
    if is_empty {
        let inside = Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(4), 1);
        f.render_widget(Paragraph::new(empty.to_string()).style(dim()), inside);
    }
}

impl App {
    pub(super) fn switch(&mut self, to: Section) -> Action {
        self.section = to;
        match to {
            Section::Tweaks => Action::None,
            // Se leen la primera vez que se entra (tarda un momento); después, con r.
            Section::Bloat if self.bloat.is_none() => Action::LoadBloat,
            Section::Bloat => Action::None,
            Section::History => Action::LoadHistory,
        }
    }

    fn section_len(&self) -> usize {
        match self.section {
            Section::Tweaks => 0,
            Section::Bloat => self.bloat.as_ref().map_or(0, Vec::len),
            Section::History => self.history.len(),
        }
    }

    pub(super) fn on_key_section(&mut self, key: KeyEvent) -> Action {
        let len = self.section_len();
        let reload = if self.section == Section::Bloat { Action::LoadBloat } else { Action::LoadHistory };
        let row = if self.section == Section::Bloat { &mut self.bloat_row } else { &mut self.history_row };
        let mut step = |delta: isize| *row = (*row as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('r') => return reload,
            KeyCode::Up | KeyCode::Char('k') => step(-1),
            KeyCode::Down | KeyCode::Char('j') => step(1),
            KeyCode::PageUp => step(-10),
            KeyCode::PageDown => step(10),
            KeyCode::Home => step(isize::MIN / 2),
            KeyCode::End => step(isize::MAX / 2),
            KeyCode::Esc => self.message = Some(tr("Para salir: q.").into()),
            KeyCode::Char('?') => self.modal = Modal::Help,
            _ if self.section == Section::Bloat => self.on_key_bloat(key),
            _ => self.on_key_history(key),
        }
        Action::None
    }

    fn on_key_bloat(&mut self, key: KeyEvent) {
        let Some(id) = self.bloat.as_ref().and_then(|b| b.get(self.bloat_row)).map(|a| a.id.clone()) else { return };
        match key.code {
            KeyCode::Char(' ') | KeyCode::Enter if !self.writable => self.message = Some(read_only()),
            KeyCode::Char(' ') => {
                if self.bloat_marked.contains(&id) {
                    self.bloat_marked.retain(|m| m != &id);
                } else {
                    self.bloat_marked.push(id);
                }
            }
            // Sin nada marcado, Enter es "esta".
            KeyCode::Enter => {
                let ids = if self.bloat_marked.is_empty() { vec![id] } else { self.bloat_marked.clone() };
                self.modal = Modal::ConfirmRemove { ids };
            }
            KeyCode::Char('c') => self.bloat_marked.clear(),
            _ => {}
        }
    }

    fn on_key_history(&mut self, key: KeyEvent) {
        if !matches!(key.code, KeyCode::Enter | KeyCode::Char('u')) {
            return;
        }
        let Some(e) = self.history.get(self.history_row) else { return };
        if !self.writable {
            self.message = Some(read_only());
        } else if e.undone {
            self.message = Some(tr("Esa entrada ya se deshizo.").into());
        } else if !journal::undoable(e) {
            self.message = Some(tr("Las apps quitadas no vuelven desde acá: se reinstalan desde Microsoft Store.").into());
        } else {
            self.modal = Modal::ConfirmUndo { id: e.id, label: e.label.clone() };
        }
    }

    // ───────────────────────── Lo que `run` le devuelve ─────────────────────────

    pub fn bloat_loading(&mut self) {
        self.bloat = None;
    }

    pub fn set_bloat(&mut self, list: Result<Vec<BloatView>, String>) {
        match list {
            Ok(apps) => {
                self.bloat_marked.retain(|m| apps.iter().any(|a| &a.id == m));
                self.bloat_row = self.bloat_row.min(apps.len().saturating_sub(1));
                self.bloat = Some(apps);
            }
            Err(e) => {
                self.bloat = Some(Vec::new());
                self.message = Some(e);
            }
        }
    }

    pub fn bloat_removed(&mut self, result: Result<(Vec<RemoveResult>, Option<String>), String>, list: Result<Vec<BloatView>, String>) {
        let title: String = tr("Apps quitadas").into();
        let name = |id: &str| self.bloat.as_deref().unwrap_or_default().iter().find(|a| a.id == id).map_or_else(|| id.to_string(), |a| a.title.clone());
        let modal = match result {
            Err(e) => Modal::Outcome { title, lines: vec![(false, e)], note: None },
            Ok((results, warning)) => Modal::Outcome {
                title,
                lines: results
                    .iter()
                    .map(|r| match &r.error {
                        None => (true, name(&r.id)),
                        Some(e) => (false, format!("{}: {e}", name(&r.id))),
                    })
                    .collect(),
                note: warning,
            },
        };
        self.bloat_marked.clear();
        self.set_bloat(list);
        self.modal = modal;
    }

    pub fn set_history(&mut self, entries: Vec<JournalEntry>) {
        self.history_row = self.history_row.min(entries.len().saturating_sub(1));
        self.history = entries;
    }

    // ───────────────────────── Dibujo ─────────────────────────

    pub(super) fn section_footer(&self) -> Line<'static> {
        let key = |k: &'static str| Span::styled(k, fg(Color::Cyan));
        let text = |t: String| Span::styled(t, dim());
        let mut spans = vec![Span::raw(" ")];
        if self.section == Section::Bloat {
            let n = self.bloat_marked.len();
            let enter = if n == 0 { tr(" quitar  ").to_string() } else { format!(" {} ({n})  ", tr("quitar lo marcado")) };
            spans.extend([key(tr("espacio")), text(tr(" marcar  ").into()), key("enter"), text(enter)]);
        } else {
            spans.extend([key("enter"), text(tr(" deshacer  ").into())]);
        }
        spans.extend([key("r"), text(tr(" releer  ").into()), key("1 2 3"), text(tr(" secciones  ").into()), key("?"), text(tr(" ayuda  ").into()), key("q"), text(tr(" salir").into())]);
        Line::from(spans)
    }

    pub(super) fn draw_section(&mut self, f: &mut Frame, body: Rect, detail: Rect) {
        let (lines, wrap) = if self.section == Section::Bloat { (self.draw_bloat(f, body), true) } else { (self.draw_history(f, body), false) };
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: wrap }).block(Block::bordered().border_style(dim())), detail);
    }

    /// Dibuja la lista de apps y devuelve el detalle de la seleccionada.
    fn draw_bloat(&mut self, f: &mut Frame, body: Rect) -> Vec<Line<'static>> {
        let apps = self.bloat.as_deref().unwrap_or_default();
        let rows: Vec<ListItem<'static>> = apps
            .iter()
            .map(|a| {
                let marked = self.bloat_marked.contains(&a.id);
                let mark = if marked { Span::styled("- ", fg(Color::Yellow).add_modifier(Modifier::BOLD)) } else { Span::styled("· ", dim()) };
                let mut spans = vec![mark, Span::styled(a.title.clone(), if marked { bold() } else { Style::new() })];
                match a.tier {
                    Risk::Safe => {}
                    Risk::Moderate => spans.push(Span::styled(format!("  {}", tr("moderada")), fg(Color::Yellow))),
                    Risk::Advanced => spans.push(Span::styled(format!("  {}", tr("avanzada")), fg(Color::Red))),
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let detail = apps
            .get(self.bloat_row)
            .map(|a| {
                let packages: Vec<String> = a.packages.iter().map(|p| p.full_name.clone()).collect();
                vec![Line::styled(a.id.clone(), fg(Color::Cyan)), Line::raw(a.note.clone().unwrap_or_default()), Line::styled(packages.join("  "), dim())]
            })
            .unwrap_or_default();
        let empty = if self.bloat.is_none() { tr("Leyendo las apps instaladas…") } else { tr("No queda ninguna app del catálogo instalada.") };
        render_list(f, body, tr("Apps de Windows que se pueden quitar"), rows, &mut self.bloat_state, self.bloat_row, empty);
        detail
    }

    /// Dibuja el historial y devuelve el detalle de la entrada seleccionada.
    fn draw_history(&mut self, f: &mut Frame, body: Rect) -> Vec<Line<'static>> {
        let when: Vec<String> = self.history.iter().map(|e| crate::ui::ago(e.at)).collect();
        let width = when.iter().map(|w| crate::ui::width(w)).max().unwrap_or(0);
        let rows: Vec<ListItem<'static>> = self
            .history
            .iter()
            .zip(&when)
            .map(|(e, when)| {
                let off = e.undone || !journal::undoable(e);
                let mut spans = vec![Span::styled(crate::ui::pad(when, width + 2), dim()), Span::styled(e.label.clone(), if off { dim() } else { Style::new() })];
                if e.undone {
                    spans.push(Span::styled(format!("  {}", tr("(deshecho)")), dim()));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let detail = self
            .history
            .get(self.history_row)
            .map(|e| {
                let mut lines = vec![Line::styled(e.label.clone(), bold())];
                if !e.changes.is_empty() {
                    let mut ids: Vec<&str> = e.changes.iter().map(|c| c.tweak_id.as_str()).collect();
                    ids.dedup();
                    lines.push(Line::raw(tf!("Valores del registro cambiados: {n}", n = e.changes.len())));
                    lines.push(Line::styled(ids.join("  "), fg(Color::Cyan)));
                }
                if !e.removed_apps.is_empty() {
                    lines.push(Line::styled(e.removed_apps.join("  "), dim()));
                }
                lines
            })
            .unwrap_or_default();
        render_list(f, body, tr("Historial"), rows, &mut self.history_state, self.history_row, tr("El historial está vacío."));
        detail
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{app, press, screen};
    use super::*;
    use prune_engine::appx::InstalledPkg;

    fn bloat(id: &str, title: &str, tier: Risk) -> BloatView {
        BloatView { id: id.into(), title: title.into(), tier, note: Some(format!("Nota de {title}.")), packages: vec![InstalledPkg { full_name: format!("{id}_1.0_x64"), family_name: id.into(), version: "1.0".into() }] }
    }

    fn entry(id: u64, extra: &str) -> JournalEntry {
        serde_json::from_str(&format!(r#"{{"id":{id},"at":{},"label":"Entrada {id}","changes":[]{extra}}}"#, id / 1000)).unwrap()
    }

    fn with_bloat(writable: bool) -> App {
        let mut a = app(true, writable);
        assert_eq!(press(&mut a, KeyCode::Char('2')), Action::LoadBloat);
        a.set_bloat(Ok(vec![bloat("Clipchamp.Clipchamp", "Clipchamp", Risk::Safe), bloat("Microsoft.Windows.Photos", "Fotos", Risk::Advanced)]));
        a
    }

    #[test]
    fn las_apps_se_leen_al_entrar_la_primera_vez() {
        let mut a = with_bloat(true);
        // Ya leídas: volver a la sección no las relee; r sí.
        assert_eq!(press(&mut a, KeyCode::Char('1')), Action::None);
        assert_eq!(press(&mut a, KeyCode::Char('2')), Action::None);
        assert_eq!(press(&mut a, KeyCode::Char('r')), Action::LoadBloat);
    }

    #[test]
    fn quitar_apps_pide_confirmacion() {
        let mut a = with_bloat(true);
        // Sin marcas, Enter es la seleccionada.
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert!(matches!(&a.modal, Modal::ConfirmRemove { ids } if ids == &["Microsoft.Windows.Photos"]));
        assert_eq!(press(&mut a, KeyCode::Esc), Action::None);
        // Con marcas, Enter es lo marcado (no la seleccionada).
        press(&mut a, KeyCode::Up);
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert!(screen(&mut a, 100, 30).contains("Clipchamp"));
        assert_eq!(press(&mut a, KeyCode::Enter), Action::RemoveBloat(vec!["Clipchamp.Clipchamp".into()]));
    }

    #[test]
    fn despues_de_quitar_se_muestra_el_resultado() {
        let mut a = with_bloat(true);
        press(&mut a, KeyCode::Char(' '));
        let results = vec![RemoveResult { id: "Clipchamp.Clipchamp".into(), ok: true, removed: vec!["Clipchamp.Clipchamp_1.0_x64".into()], error: None }];
        a.bloat_removed(Ok((results, None)), Ok(vec![bloat("Microsoft.Windows.Photos", "Fotos", Risk::Advanced)]));
        assert!(a.bloat_marked.is_empty());
        let s = screen(&mut a, 100, 30);
        assert!(s.contains("✓ Clipchamp"), "{s}");
        press(&mut a, KeyCode::Char('x'));
        assert!(screen(&mut a, 100, 30).contains("Fotos"));
    }

    #[test]
    fn en_solo_lectura_no_se_quita_nada() {
        let mut a = with_bloat(false);
        press(&mut a, KeyCode::Char(' '));
        assert_eq!(press(&mut a, KeyCode::Enter), Action::None);
        assert!(a.bloat_marked.is_empty() && matches!(a.modal, Modal::None));
        assert!(a.message.is_some());
    }

    #[test]
    fn el_historial_deshace_la_entrada_elegida() {
        let mut a = app(true, true);
        assert_eq!(press(&mut a, KeyCode::Char('3')), Action::LoadHistory);
        a.set_history(vec![entry(3000, r#","removed_apps":["A_1"]"#), entry(2000, r#","undone":true,"power":{"before":"a","after":"b"}"#), entry(1000, r#","power":{"before":"a","after":"b"}"#)]);
        let s = screen(&mut a, 100, 30);
        assert!(s.contains("Entrada 3000") && s.contains("(deshecho)"), "{s}");
        // Apps quitadas y entradas ya deshechas: no hay qué deshacer.
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.modal, Modal::None) && a.message.is_some());
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.modal, Modal::None));
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.modal, Modal::ConfirmUndo { id: 1000, .. }));
        assert_eq!(press(&mut a, KeyCode::Enter), Action::Undo(1000));
    }

    #[test]
    fn las_secciones_no_rompen_en_ventana_chica() {
        let mut a = with_bloat(true);
        screen(&mut a, 60, 14);
        press(&mut a, KeyCode::Char('3'));
        screen(&mut a, 60, 14);
        // Y la búsqueda de ajustes no se activa fuera de su sección.
        press(&mut a, KeyCode::Char('/'));
        assert!(!a.typing);
    }
}
