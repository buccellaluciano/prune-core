//! Interfaz interactiva: lo que abre `prune` sin argumentos.
//!
//! Misma regla que el resto de P-Rune: nada se cambia sin ver antes el plan. Acá se marcan
//! ajustes, Enter muestra qué valores van a cambiar y recién ahí se confirma.
//!
//! `App` es solo estado y dibujo (se puede probar sin terminal ni registro); lo que toca el
//! sistema pasa en `run`, que es quien habla con el motor.

use crate::ui::tr;
use crate::{Ctx, bloat, journal, tweaks};
use prune_engine::appx::{self, BloatView};
use prune_engine::catalog::{self, Risk};
use prune_engine::engine::{self, ApplyReport, Desired, JournalEntry, PlannedChange, TweakState, TweakView};
use prune_engine::i18n;
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

// Las secciones de apps de Windows e historial (mismo `App`, en otro archivo).
#[path = "tui_sections.rs"]
mod sections;
use sections::Section;

/// Lo que `App` le pide a `run` después de una tecla (todo lo que necesita al motor).
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    /// Calcular el plan de lo marcado y mostrarlo.
    Plan,
    /// Aplicar el plan que se está mostrando.
    Apply,
    /// Buscar la última entrada del historial que se pueda deshacer.
    AskUndo,
    Undo(u64),
    /// Volver a leer el estado real.
    Reload,
    /// Leer qué apps del catálogo están instaladas.
    LoadBloat,
    RemoveBloat(Vec<String>),
    LoadHistory,
}

#[derive(PartialEq, Eq)]
enum Focus {
    Cats,
    Tweaks,
}

enum Modal {
    None,
    Help,
    /// Elegir la opción de un ajuste (`view` es su posición en `views`).
    Choice { view: usize, sel: usize },
    Plan { plan: Result<Vec<PlannedChange>, String>, scroll: u16 },
    ConfirmUndo { id: u64, label: String },
    /// Quitar apps de Windows (ids del catálogo).
    ConfirmRemove { ids: Vec<String> },
    /// Resultado de aplicar o deshacer: (salió bien, texto).
    Outcome { title: String, lines: Vec<(bool, String)>, note: Option<String> },
}

pub struct App {
    header: String,
    /// (id, título) de las categorías que tienen algún ajuste, en el orden del catálogo.
    cats: Vec<(String, String)>,
    views: Vec<TweakView>,
    elevated: bool,
    writable: bool,
    focus: Focus,
    cat: usize,
    row: usize,
    cat_state: ListState,
    row_state: ListState,
    /// Texto del filtro. Con algo escrito, la lista muestra coincidencias de todas las categorías.
    search: String,
    typing: bool,
    /// Lo marcado para cambiar: a lo sumo uno por ajuste.
    pending: Vec<Desired>,
    modal: Modal,
    message: Option<String>,
    section: Section,
    /// Apps del catálogo instaladas. None: todavía no se leyeron.
    bloat: Option<Vec<BloatView>>,
    bloat_row: usize,
    bloat_state: ListState,
    bloat_marked: Vec<String>,
    /// La más nueva primero, como la devuelve el motor.
    history: Vec<JournalEntry>,
    history_row: usize,
    history_state: ListState,
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

fn fg(c: Color) -> Style {
    Style::new().fg(c)
}

fn matches(v: &TweakView, q: &str) -> bool {
    let q = crate::ui::fold(q);
    let has = |s: &str| crate::ui::fold(s).contains(&q);
    has(&v.id) || has(&v.title) || has(&v.description) || v.tags.iter().any(|t| has(t))
}

/// Un rectángulo de `w` × `h` centrado en `area` (achicado si no entra).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

impl App {
    pub fn new(header: String, cats: Vec<(String, String)>, views: Vec<TweakView>, elevated: bool, writable: bool) -> Self {
        let cats = cats.into_iter().filter(|(id, _)| views.iter().any(|v| &v.category == id)).collect();
        Self {
            header,
            cats,
            views,
            elevated,
            writable,
            focus: Focus::Tweaks,
            cat: 0,
            row: 0,
            cat_state: ListState::default(),
            row_state: ListState::default(),
            search: String::new(),
            typing: false,
            pending: Vec::new(),
            modal: Modal::None,
            message: None,
            section: Section::Tweaks,
            bloat: None,
            bloat_row: 0,
            bloat_state: ListState::default(),
            bloat_marked: Vec::new(),
            history: Vec::new(),
            history_row: 0,
            history_state: ListState::default(),
        }
    }

    pub fn pending(&self) -> &[Desired] {
        &self.pending
    }

    fn searching(&self) -> bool {
        !self.search.is_empty()
    }

    /// Posiciones en `views` de lo que muestra la lista de la derecha.
    fn visible(&self) -> Vec<usize> {
        let all = self.views.iter().enumerate();
        if self.searching() {
            let q = self.search.to_lowercase();
            return all.filter(|(_, v)| matches(v, &q)).map(|(i, _)| i).collect();
        }
        let cat = self.cats.get(self.cat).map(|c| c.0.as_str());
        all.filter(|(_, v)| Some(v.category.as_str()) == cat).map(|(i, _)| i).collect()
    }

    fn selected(&self) -> Option<usize> {
        self.visible().get(self.row).copied()
    }

    fn pending_for(&self, id: &str) -> Option<&Desired> {
        self.pending.iter().find(|d| d.id == id)
    }

    fn locked(&self, v: &TweakView) -> bool {
        v.requires_admin && !self.elevated
    }

    fn step(&mut self, delta: isize) {
        let clamp = |i: usize, len: usize| (i as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
        if self.focus == Focus::Cats && !self.searching() {
            let next = clamp(self.cat, self.cats.len());
            if next != self.cat {
                self.cat = next;
                self.row = 0;
                self.row_state = ListState::default();
            }
        } else {
            self.row = clamp(self.row, self.visible().len());
        }
    }

    fn set_search(&mut self, text: String) {
        self.search = text;
        self.row = 0;
        self.row_state = ListState::default();
    }

    /// Marca o desmarca el ajuste seleccionado. Si tiene opciones, abre el selector.
    fn toggle(&mut self) {
        let Some(i) = self.selected() else { return };
        let v = &self.views[i];
        if !self.writable {
            self.message = Some(
                tr("Solo lectura: P-Rune está instalado para todo el equipo. Abrí una terminal de administrador para cambiar algo.")
                .into(),
            );
            return;
        }
        if matches!(v.state, TweakState::Unsupported | TweakState::Error) {
            self.message = Some(format!("{}: {}", v.title, tweaks::state_text(v.state)));
            return;
        }
        if self.locked(v) {
            self.message = Some(format!("{}: {}", v.title, tr("pide una terminal de administrador.")));
            return;
        }
        if !v.choices.is_empty() {
            let current = self.pending_for(&v.id).and_then(|d| d.choice.clone()).or_else(|| v.choice.clone());
            let sel = v.choices.iter().position(|c| Some(&c.id) == current.as_ref()).unwrap_or(0);
            self.modal = Modal::Choice { view: i, sel };
            return;
        }
        let id = v.id.clone();
        if self.pending_for(&id).is_some() {
            self.pending.retain(|d| d.id != id);
        } else {
            // A medias cuenta como sin aplicar: marcarlo lo completa.
            self.pending.push(Desired { id, applied: v.state != TweakState::Applied, choice: None });
        }
    }

    fn pick_choice(&mut self, view: usize, sel: usize) {
        let v = &self.views[view];
        let Some(c) = v.choices.get(sel) else { return };
        let (id, choice, default) = (v.id.clone(), c.id.clone(), c.default);
        let already = v.choice.as_deref() == Some(choice.as_str());
        self.pending.retain(|d| d.id != id);
        // Elegir la que ya está puesta es desmarcar.
        if !already {
            self.pending.push(Desired { id, applied: !default, choice: Some(choice) });
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        self.message = None;
        match &mut self.modal {
            Modal::None => {}
            Modal::Help | Modal::Outcome { .. } => {
                self.modal = Modal::None;
                return Action::None;
            }
            Modal::Choice { view, sel } => {
                let (view, len) = (*view, self.views[*view].choices.len());
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *sel = sel.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => *sel = (*sel + 1).min(len.saturating_sub(1)),
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        let sel = *sel;
                        self.modal = Modal::None;
                        self.pick_choice(view, sel);
                    }
                    KeyCode::Esc | KeyCode::Char('q') => self.modal = Modal::None,
                    _ => {}
                }
                return Action::None;
            }
            Modal::Plan { plan, scroll } => {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
                    KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('s') => {
                        if plan.as_ref().is_ok_and(|p| p.iter().any(|c| !c.noop)) {
                            return Action::Apply;
                        }
                        self.modal = Modal::None;
                    }
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => self.modal = Modal::None,
                    _ => {}
                }
                return Action::None;
            }
            Modal::ConfirmUndo { id, .. } => {
                let id = *id;
                self.modal = Modal::None;
                return match key.code {
                    KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('s') => Action::Undo(id),
                    _ => Action::None,
                };
            }
            Modal::ConfirmRemove { ids } => {
                let ids = std::mem::take(ids);
                self.modal = Modal::None;
                return match key.code {
                    KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('s') => Action::RemoveBloat(ids),
                    _ => Action::None,
                };
            }
        }

        if self.typing {
            match key.code {
                KeyCode::Char(c) => {
                    let mut s = self.search.clone();
                    s.push(c);
                    self.set_search(s);
                }
                KeyCode::Backspace => {
                    let mut s = self.search.clone();
                    s.pop();
                    self.set_search(s);
                }
                KeyCode::Enter => self.typing = false,
                KeyCode::Esc => {
                    self.typing = false;
                    self.set_search(String::new());
                }
                KeyCode::Up => self.step(-1),
                KeyCode::Down => self.step(1),
                _ => {}
            }
            return Action::None;
        }

        match key.code {
            KeyCode::Char('1') => return self.switch(Section::Tweaks),
            KeyCode::Char('2') => return self.switch(Section::Bloat),
            KeyCode::Char('3') => return self.switch(Section::History),
            _ => {}
        }
        if self.section != Section::Tweaks {
            return self.on_key_section(key);
        }

        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Esc => {
                if self.searching() {
                    self.set_search(String::new());
                } else {
                    self.message = Some(tr("Para salir: q.").into());
                }
            }
            KeyCode::Char('/') => {
                self.typing = true;
                self.focus = Focus::Tweaks;
            }
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::PageUp => self.step(-10),
            KeyCode::PageDown => self.step(10),
            KeyCode::Home => self.step(isize::MIN / 2),
            KeyCode::End => self.step(isize::MAX / 2),
            KeyCode::Left | KeyCode::Char('h') if !self.searching() => self.focus = Focus::Cats,
            KeyCode::Right | KeyCode::Char('l') => self.focus = Focus::Tweaks,
            KeyCode::Tab if !self.searching() => {
                self.focus = if self.focus == Focus::Cats { Focus::Tweaks } else { Focus::Cats };
            }
            KeyCode::Char(' ') => {
                if self.focus == Focus::Cats && !self.searching() {
                    self.focus = Focus::Tweaks;
                } else {
                    self.toggle();
                }
            }
            KeyCode::Enter => {
                if self.focus == Focus::Cats && !self.searching() {
                    self.focus = Focus::Tweaks;
                } else {
                    // Sin nada marcado, Enter es "este": lo marca y muestra su plan.
                    if self.pending.is_empty() {
                        self.toggle();
                    }
                    if !self.pending.is_empty() && matches!(self.modal, Modal::None) {
                        return Action::Plan;
                    }
                }
            }
            KeyCode::Char('c') => {
                if !self.pending.is_empty() {
                    self.pending.clear();
                    self.message = Some(tr("Marcas borradas.").into());
                }
            }
            KeyCode::Char('u') => return Action::AskUndo,
            KeyCode::Char('r') => return Action::Reload,
            _ => {}
        }
        Action::None
    }

    // ───────────────────────── Lo que `run` le devuelve ─────────────────────────

    pub fn set_views(&mut self, views: Vec<TweakView>) {
        self.views = views;
        self.pending.retain(|d| self.views.iter().any(|v| v.id == d.id));
        self.row = self.row.min(self.visible().len().saturating_sub(1));
    }

    pub fn set_message(&mut self, text: String) {
        self.message = Some(text);
    }

    pub fn show_plan(&mut self, plan: Result<Vec<PlannedChange>, String>) {
        self.modal = Modal::Plan { plan, scroll: 0 };
    }

    /// Lo marcado que de verdad cambia algo según el plan en pantalla (lo que ya está así
    /// no se aplica ni ensucia el historial).
    pub fn changing(&self) -> Vec<Desired> {
        let Modal::Plan { plan: Ok(plan), .. } = &self.modal else { return Vec::new() };
        self.pending.iter().filter(|d| plan.iter().any(|p| p.tweak_id == d.id && !p.noop)).cloned().collect()
    }

    fn title_of(&self, id: &str) -> String {
        self.views.iter().find(|v| v.id == id).map_or_else(|| id.to_string(), |v| v.title.clone())
    }

    pub fn applied(&mut self, report: &ApplyReport, views: Vec<TweakView>) {
        let lines = report
            .results
            .iter()
            .map(|r| match &r.error {
                None => (true, self.title_of(&r.id)),
                Some(e) => (false, format!("{}: {e}", self.title_of(&r.id))),
            })
            .collect();
        let mut note: Vec<String> = Vec::new();
        if let Some(w) = &report.warning {
            note.push(w.clone());
        }
        if let Some(r) = tweaks::restart_text(report.restart) {
            note.push(tf!("Para que se vea hace falta: {r}.", r = r));
        }
        if report.entry_id.is_some() {
            note.push(tr("Para deshacerlo: u.").into());
        }
        self.pending.clear();
        self.set_views(views);
        self.modal =
            Modal::Outcome { title: tr("Aplicado").into(), lines, note: (!note.is_empty()).then(|| note.join(" ")) };
    }

    pub fn ask_undo(&mut self, id: u64, label: String) {
        self.modal = Modal::ConfirmUndo { id, label };
    }

    pub fn undone(&mut self, result: Result<ApplyReport, String>, views: Vec<TweakView>) {
        let title: String = tr("Deshacer").into();
        self.set_views(views);
        self.modal = match result {
            Err(e) => Modal::Outcome { title, lines: vec![(false, e)], note: None },
            Ok(r) if !r.conflicts.is_empty() => Modal::Outcome {
                title,
                lines: r.conflicts.iter().map(|c| (false, c.clone())).collect(),
                note: Some(
                    tr("Esos valores cambiaron después: no se tocó nada. Para deshacer igual: prune undo --force.")
                    .into(),
                ),
            },
            Ok(r) => {
                let failed: Vec<(bool, String)> =
                    r.results.iter().filter(|x| !x.ok).map(|x| (false, format!("{}: {}", x.id, x.error.clone().unwrap_or_default()))).collect();
                let lines = if failed.is_empty() { vec![(true, tr("Deshecho.").to_string())] } else { failed };
                Modal::Outcome { title, lines, note: r.warning }
            }
        };
    }

    // ───────────────────────── Dibujo ─────────────────────────

    fn tweak_line(&self, v: &TweakView, with_category: bool) -> Line<'static> {
        let pending = self.pending_for(&v.id);
        let off = v.state == TweakState::Unsupported || self.locked(v);
        let mark = match (pending, v.state) {
            (Some(d), _) if d.choice.is_some() => Span::styled("~ ", fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            (Some(d), _) if d.applied => Span::styled("+ ", fg(Color::Green).add_modifier(Modifier::BOLD)),
            (Some(_), _) => Span::styled("- ", fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            (None, TweakState::Applied) => Span::styled("● ", fg(Color::Green)),
            (None, TweakState::Default) => Span::styled("○ ", dim()),
            (None, TweakState::Partial) => Span::styled("◐ ", fg(Color::Yellow)),
            (None, TweakState::Unsupported) => Span::styled("– ", dim()),
            (None, TweakState::Error) => Span::styled("! ", fg(Color::Red)),
        };
        let title_style = if pending.is_some() {
            bold()
        } else if off {
            dim()
        } else {
            Style::new()
        };
        let mut spans = vec![mark, Span::styled(v.title.clone(), title_style)];
        let label = |id: &str| v.choices.iter().find(|c| c.id == id).map(|c| c.label.clone());
        if let Some(l) = v.choice.as_deref().and_then(label) {
            spans.push(Span::styled(format!(" = {l}"), dim()));
        }
        if let Some(l) = pending.and_then(|d| d.choice.as_deref()).and_then(label) {
            spans.push(Span::styled(format!(" → {l}"), fg(Color::Yellow)));
        }
        match v.risk {
            Risk::Safe => {}
            Risk::Moderate => spans.push(Span::styled(format!("  {}", tweaks::risk_text(v.risk)), fg(Color::Yellow))),
            Risk::Advanced => spans.push(Span::styled(format!("  {}", tweaks::risk_text(v.risk)), fg(Color::Red))),
        }
        if v.requires_admin {
            spans.push(Span::styled("  admin", dim()));
        }
        if with_category {
            let cat = self.cats.iter().find(|c| c.0 == v.category).map_or(v.category.as_str(), |c| c.1.as_str());
            spans.push(Span::styled(format!("  · {cat}"), dim()));
        }
        Line::from(spans)
    }

    fn detail(&self, v: &TweakView) -> Vec<Line<'static>> {
        let mut head = vec![Span::styled(v.id.clone(), fg(Color::Cyan)), Span::raw("  "), Span::raw(tweaks::state_text(v.state))];
        if v.risk != Risk::Safe {
            head.push(Span::styled(format!("  {} {}", tr("riesgo"), tweaks::risk_text(v.risk)), dim()));
        }
        if let Some(r) = tweaks::restart_text(v.restart) {
            head.push(Span::styled(format!("  {}", tf!("hace falta: {r}", r = r)), dim()));
        }
        let mut lines = vec![Line::from(head), Line::from(v.description.clone())];
        for op in v.ops.iter().take(3) {
            lines.push(Line::styled(format!("{}\\{}: {}", op.key, op.name, op.current), dim()));
        }
        lines
    }

    fn footer(&self) -> Line<'static> {
        if let Some(m) = &self.message {
            return Line::styled(format!(" {m}"), fg(Color::Yellow));
        }
        if self.typing {
            return Line::styled(tr(" escribí para filtrar · enter listo · esc borrar"), dim());
        }
        if self.section != Section::Tweaks {
            return self.section_footer();
        }
        let n = self.pending.len();
        let key = |k: &'static str| Span::styled(k, fg(Color::Cyan));
        let text = |t: String| Span::styled(t, dim());
        let enter = if n == 0 { tr(" aplicar  ").to_string() } else { format!(" {} ({n})  ", tr("revisar y aplicar")) };
        Line::from(vec![
            Span::raw(" "),
            key(tr("espacio")),
            text(tr(" marcar  ").into()),
            key("enter"),
            text(enter),
            key("/"),
            text(tr(" buscar  ").into()),
            key("u"),
            text(tr(" deshacer  ").into()),
            key("?"),
            text(tr(" ayuda  ").into()),
            key("q"),
            text(tr(" salir").into()),
        ])
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        if area.width < 60 || area.height < 14 {
            f.render_widget(Paragraph::new(tr("Agrandá la ventana para usar P-Rune.")), area);
            return;
        }
        let [header, body, detail, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(5), Constraint::Length(7), Constraint::Length(1)]).areas(area);
        let cat_width = self.cats.iter().map(|c| crate::ui::width(&c.1)).max().unwrap_or(10).clamp(12, 28) as u16 + 10;
        let [left, right] = Layout::horizontal([Constraint::Length(cat_width), Constraint::Min(20)]).areas(body);

        let tab = |n: &str, name: &str, s: Section| {
            let style = if self.section == s { Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD) } else { dim() };
            Span::styled(format!(" {n} {name} "), style)
        };
        let tabs = vec![
            Span::styled(" P-Rune Core ", bold()),
            tab("1", tr("Ajustes"), Section::Tweaks),
            tab("2", tr("Apps de Windows"), Section::Bloat),
            tab("3", tr("Historial"), Section::History),
            Span::raw("  "),
            Span::styled(self.header.clone(), dim()),
        ];
        f.render_widget(Paragraph::new(Line::from(tabs)), header);
        if self.section != Section::Tweaks {
            self.draw_section(f, body, detail);
            f.render_widget(Paragraph::new(self.footer()), footer);
            self.draw_modal(f, area);
            return;
        }

        let border = |focused: bool| if focused { fg(Color::Cyan) } else { dim() };
        let highlight = |focused: bool| if focused { Style::new().add_modifier(Modifier::REVERSED) } else { bold() };
        let cats_focused = self.focus == Focus::Cats && !self.searching();

        // Categorías: nombre, cuántos aplicados y un asterisco si tiene algo marcado.
        let inner = left.width.saturating_sub(4) as usize;
        let items: Vec<ListItem> = self
            .cats
            .iter()
            .map(|(id, title)| {
                let in_cat: Vec<&TweakView> = self.views.iter().filter(|v| &v.category == id && v.state != TweakState::Unsupported).collect();
                let on = in_cat.iter().filter(|v| v.state == TweakState::Applied).count();
                let marked = self.pending.iter().any(|d| self.views.iter().any(|v| v.id == d.id && &v.category == id));
                let count = format!("{}{on}/{}", if marked { "*" } else { "" }, in_cat.len());
                let gap = inner.saturating_sub(crate::ui::width(title) + crate::ui::width(&count)).max(1);
                let count_style = if marked { fg(Color::Yellow) } else if on > 0 { fg(Color::Green) } else { dim() };
                ListItem::new(Line::from(vec![Span::raw(title.clone()), Span::raw(" ".repeat(gap)), Span::styled(count, count_style)]))
            })
            .collect();
        let cats_style = if self.searching() { dim() } else { Style::new() };
        let list = List::new(items)
            .style(cats_style)
            .block(Block::bordered().border_style(border(cats_focused)).title(tr(" Categorías ")))
            .highlight_style(highlight(cats_focused))
            .highlight_symbol(" ");
        self.cat_state.select(Some(self.cat));
        f.render_stateful_widget(list, left, &mut self.cat_state);

        // Ajustes de la categoría, o las coincidencias de la búsqueda.
        let visible = self.visible();
        let title = if self.typing || self.searching() {
            format!(" {} {}{} ", tr("Buscar:"), self.search, if self.typing { "▏" } else { "" })
        } else {
            format!(" {} ", self.cats.get(self.cat).map_or("", |c| c.1.as_str()))
        };
        let rows: Vec<ListItem> = visible.iter().map(|&i| ListItem::new(self.tweak_line(&self.views[i], self.searching()))).collect();
        let empty = rows.is_empty();
        let list = List::new(rows)
            .block(Block::bordered().border_style(border(!cats_focused)).title(title))
            .highlight_style(highlight(!cats_focused))
            .highlight_symbol(" ");
        self.row_state.select((!empty).then_some(self.row));
        f.render_stateful_widget(list, right, &mut self.row_state);
        if empty {
            let inside = Rect::new(right.x + 2, right.y + 1, right.width.saturating_sub(4), 1);
            f.render_widget(Paragraph::new(tr("Ningún ajuste coincide.")).style(dim()), inside);
        }

        let lines = self.selected().map(|i| self.detail(&self.views[i])).unwrap_or_default();
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }).block(Block::bordered().border_style(dim())), detail);
        f.render_widget(Paragraph::new(self.footer()), footer);

        self.draw_modal(f, area);
    }

    fn draw_modal(&self, f: &mut Frame, area: Rect) {
        let popup = |f: &mut Frame, rect: Rect, title: String, hint: &str, lines: Vec<Line<'static>>, scroll: u16| {
            f.render_widget(Clear, rect);
            let block = Block::bordered()
                .border_style(fg(Color::Cyan))
                .title(format!(" {title} "))
                .title_bottom(Line::styled(format!(" {hint} "), dim()));
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)).block(block), rect);
        };
        let outcome = |ok: bool, text: &str| {
            let mark = if ok { Span::styled("✓ ", fg(Color::Green)) } else { Span::styled("✗ ", fg(Color::Red)) };
            Line::from(vec![mark, Span::raw(text.to_string())])
        };
        let wide = area.width.saturating_sub(8).min(110);
        match &self.modal {
            Modal::None => {}
            Modal::Help => {
                let keys: [(&str, &str); 11] = [
                    ("1 2 3", tr("cambiar de sección: ajustes, apps de Windows, historial")),
                    ("↑ ↓  j k", tr("moverse")),
                    ("← →  tab", tr("cambiar entre categorías y ajustes")),
                    (tr("espacio"), tr("marcar o desmarcar (o elegir una opción)")),
                    ("enter", tr("ver el plan de lo marcado y aplicar")),
                    ("/", tr("buscar en todas las categorías")),
                    ("c", tr("borrar las marcas")),
                    ("u", tr("deshacer lo último del historial")),
                    ("r", tr("volver a leer el estado")),
                    ("?", tr("esta ayuda")),
                    ("q", tr("salir (lo marcado sin aplicar se descarta)")),
                ];
                let mut lines: Vec<Line> =
                    keys.iter().map(|(k, d)| Line::from(vec![Span::styled(format!("{k:<10}"), fg(Color::Cyan)), Span::raw(d.to_string())])).collect();
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    tr("● aplicado  ○ como lo trae Windows  ◐ a medias  – no disponible"),
                    dim(),
                ));
                lines.push(Line::styled(tr("+ se va a aplicar  - se va a revertir  ~ cambia de opción"), dim()));
                let h = lines.len() as u16 + 2;
                popup(f, centered(area, 70, h), tr("Teclas").into(), tr("cualquier tecla para volver"), lines, 0);
            }
            Modal::Choice { view, sel } => {
                let v = &self.views[*view];
                let lines: Vec<Line> = v
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let current = v.choice.as_deref() == Some(c.id.as_str());
                        let mark = if current { Span::styled("● ", fg(Color::Green)) } else { Span::styled("○ ", dim()) };
                        let mut spans = vec![Span::raw(if i == *sel { "› " } else { "  " }), mark, Span::raw(c.label.clone())];
                        if c.default {
                            spans.push(Span::styled(tr("  (la de Windows)"), dim()));
                        }
                        let line = Line::from(spans);
                        if i == *sel { line.style(bold()) } else { line }
                    })
                    .collect();
                let h = lines.len() as u16 + 2;
                popup(f, centered(area, 56, h), v.title.clone(), tr("enter elegir · esc volver"), lines, 0);
            }
            Modal::Plan { plan, scroll } => {
                let (lines, hint): (Vec<Line>, &str) = match plan {
                    Err(e) => (vec![outcome(false, e)], tr("esc volver")),
                    Ok(plan) => {
                        let mut lines = Vec::new();
                        let mut last = "";
                        for p in plan {
                            if p.tweak_id != last {
                                last = &p.tweak_id;
                                lines.push(Line::from(vec![
                                    Span::styled(self.title_of(&p.tweak_id), bold()),
                                    Span::styled(format!("  {}", p.tweak_id), fg(Color::Cyan)),
                                ]));
                            }
                            lines.push(if p.noop {
                                Line::styled(format!("  {}\\{}: {} ({})", p.key, p.name, p.from, tr("ya está así")), dim())
                            } else {
                                Line::from(vec![
                                    Span::raw(format!("  {}\\{}: ", p.key, p.name)),
                                    Span::styled(p.from.clone(), dim()),
                                    Span::styled(" → ", dim()),
                                    Span::styled(p.to.clone(), fg(Color::Green)),
                                ])
                            });
                        }
                        if plan.iter().any(|p| !p.noop) {
                            (lines, tr("enter aplicar · esc volver · ↑↓ desplazar"))
                        } else {
                            lines.push(Line::raw(""));
                            lines.push(Line::raw(tr("Ya está todo así: no hay nada que cambiar.")));
                            (lines, tr("esc volver"))
                        }
                    }
                };
                let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(4));
                popup(f, centered(area, wide, h), tr("Plan: esto es lo que va a cambiar").into(), hint, lines, *scroll);
            }
            Modal::ConfirmRemove { ids } => {
                let title_of = |id: &String| self.bloat.as_deref().unwrap_or_default().iter().find(|a| &a.id == id).map_or_else(|| id.clone(), |a| a.title.clone());
                let mut lines: Vec<Line> = ids.iter().map(|id| Line::styled(title_of(id), bold())).collect();
                lines.push(Line::raw(""));
                lines.push(Line::raw(tr("No se puede deshacer desde P-Rune: se reinstalan desde Microsoft Store.")));
                let h = (lines.len() as u16 + 3).min(area.height.saturating_sub(4));
                popup(f, centered(area, wide.min(84), h), tf!("¿Quitar {n} apps?", n = ids.len()), tr("enter quitar · esc volver"), lines, 0);
            }
            Modal::ConfirmUndo { label, .. } => {
                let lines = vec![Line::styled(label.clone(), bold()), Line::raw(""), Line::raw(tr("Vuelve cada valor a como estaba antes."))];
                popup(f, centered(area, wide.min(72), 5), tr("¿Deshacer?").into(), tr("enter deshacer · esc volver"), lines, 0);
            }
            Modal::Outcome { title, lines, note } => {
                let mut out: Vec<Line> = lines.iter().map(|(ok, t)| outcome(*ok, t)).collect();
                if let Some(n) = note {
                    out.push(Line::raw(""));
                    out.push(Line::styled(n.clone(), dim()));
                }
                // La nota puede ocupar más de un renglón.
                let h = (out.len() as u16 + 4).min(area.height.saturating_sub(4));
                popup(f, centered(area, wide.min(90), h), title.clone(), tr("cualquier tecla para seguir"), out, 0);
            }
        }
    }
}

// ───────────────────────── Terminal y motor ─────────────────────────

pub fn run(ctx: &Ctx) -> Result<(), String> {
    let i = &ctx.info;
    let header = format!(
        "{} {} · {}",
        i.product,
        i.display_version,
        if i.elevated { tr("administrador") } else { tr("sin administrador") }
    );
    let cats = catalog::tweaks().category.iter().map(|c| (c.id.clone(), i18n::category(&c.id, &c.title))).collect();
    let mut app = App::new(header, cats, engine::list(i.build), i.elevated, ctx.writable);

    let mut terminal = ratatui::try_init().map_err(|e| e.to_string())?;
    let result = event_loop(&mut terminal, &mut app, ctx);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App, ctx: &Ctx) -> Result<(), String> {
    let (build, foreign) = (ctx.info.build, ctx.info.foreign_user.as_deref());
    loop {
        terminal.draw(|f| app.draw(f)).map_err(|e| e.to_string())?;
        // Cualquier otro evento (cambio de tamaño, foco) solo vuelve a dibujar.
        let Event::Key(key) = event::read().map_err(|e| e.to_string())? else { continue };
        // En Windows llegan también las teclas al soltarse.
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match app.on_key(key) {
            Action::None => {}
            Action::Quit => return Ok(()),
            Action::Plan => app.show_plan(engine::plan(app.pending(), build)),
            Action::Apply => {
                let report = ctx.engine.apply(&app.changing(), ctx.info.elevated, build, foreign);
                app.applied(&report, engine::list(build));
            }
            Action::AskUndo => match ctx.need_write() {
                Err(e) => app.set_message(e),
                Ok(()) => match ctx.engine.history().into_iter().find(journal::undoable) {
                    Some(e) => app.ask_undo(e.id, e.label),
                    None => app.set_message(tr("No hay nada para deshacer.").into()),
                },
            },
            Action::Undo(id) => {
                let result = ctx.engine.undo(id, false, foreign);
                app.undone(result, engine::list(build));
                app.set_history(ctx.engine.history());
            }
            Action::Reload => app.set_views(engine::list(build)),
            Action::LoadBloat => {
                // Tarda un momento: primero se dibuja el aviso de que se está leyendo.
                app.bloat_loading();
                terminal.draw(|f| app.draw(f)).map_err(|e| e.to_string())?;
                app.set_bloat(appx::list());
            }
            Action::RemoveBloat(ids) => {
                let result = bloat::remove_now(ctx, &ids, false);
                app.bloat_removed(result, appx::list());
                app.set_history(ctx.engine.history());
            }
            Action::LoadHistory => app.set_history(ctx.engine.history()),
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use prune_engine::catalog::Restart;
    use prune_engine::engine::{ChoiceView, ItemResult, OpView};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn view(id: &str, title: &str, state: TweakState, admin: bool) -> TweakView {
        TweakView {
            id: id.into(),
            category: id.split('.').next().unwrap().into(),
            title: title.into(),
            description: format!("Descripción de {title}."),
            risk: if admin { Risk::Moderate } else { Risk::Safe },
            restart: Restart::None,
            tags: vec![],
            requires_admin: admin,
            per_user: !admin,
            ucpd: false,
            state,
            ops: vec![OpView { key: r"HKCU\Software\Prueba".into(), name: "Valor".into(), current: "0".into(), on: "1".into(), off: "0".into() }],
            choices: vec![],
            choice: None,
        }
    }

    pub(super) fn app(elevated: bool, writable: bool) -> App {
        let mut mode = view("search.taskbar_mode", "Búsqueda en la barra", TweakState::Default, false);
        mode.choices = vec![
            ChoiceView { id: "hidden".into(), label: "Oculta".into(), default: false },
            ChoiceView { id: "box".into(), label: "Cuadro".into(), default: true },
        ];
        mode.choice = Some("box".into());
        let views = vec![
            view("privacy.advertising_id", "Desactivar ID de publicidad", TweakState::Applied, false),
            view("privacy.telemetry_policy", "Telemetría al mínimo", TweakState::Default, true),
            view("privacy.old", "Algo viejo", TweakState::Unsupported, false),
            mode,
            view("taskbar.align_left", "Alinear a la izquierda", TweakState::Default, false),
        ];
        let cats = [("privacy", "Privacidad"), ("search", "Búsqueda"), ("taskbar", "Barra de tareas"), ("vacia", "Sin ajustes")];
        App::new("0.5.0 · Windows 11".into(), cats.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), views, elevated, writable)
    }

    pub(super) fn press(app: &mut App, code: KeyCode) -> Action {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    pub(super) fn screen(app: &mut App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn las_categorias_sin_ajustes_no_aparecen() {
        let a = app(true, true);
        assert_eq!(a.cats.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["privacy", "search", "taskbar"]);
    }

    #[test]
    fn espacio_marca_y_desmarca() {
        let mut a = app(true, true);
        // El primero está aplicado: marcarlo es revertirlo.
        press(&mut a, KeyCode::Char(' '));
        assert_eq!(a.pending.len(), 1);
        assert!(!a.pending[0].applied);
        press(&mut a, KeyCode::Char(' '));
        assert!(a.pending.is_empty());
        // El segundo no está aplicado: marcarlo es aplicarlo.
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Char(' '));
        assert_eq!((a.pending[0].id.as_str(), a.pending[0].applied), ("privacy.telemetry_policy", true));
    }

    #[test]
    fn sin_admin_no_se_marca_lo_que_pide_admin() {
        let mut a = app(false, true);
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Char(' '));
        assert!(a.pending.is_empty());
        assert!(a.message.as_ref().unwrap().contains("administrador"));
        // Y Enter tampoco arma un plan con eso.
        assert_eq!(press(&mut a, KeyCode::Enter), Action::None);
    }

    #[test]
    fn lo_no_disponible_no_se_marca() {
        let mut a = app(true, true);
        press(&mut a, KeyCode::End);
        assert_eq!(a.views[a.selected().unwrap()].id, "privacy.old");
        press(&mut a, KeyCode::Char(' '));
        assert!(a.pending.is_empty());
    }

    #[test]
    fn solo_lectura_no_marca_nada() {
        let mut a = app(true, false);
        press(&mut a, KeyCode::Char(' '));
        assert!(a.pending.is_empty());
        assert!(a.message.is_some());
    }

    #[test]
    fn enter_sin_marcas_arma_el_plan_de_ese_ajuste() {
        let mut a = app(true, true);
        assert_eq!(press(&mut a, KeyCode::Enter), Action::Plan);
        assert_eq!(a.pending[0].id, "privacy.advertising_id");
    }

    #[test]
    fn enter_con_marcas_no_agrega_el_seleccionado() {
        let mut a = app(true, true);
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Down);
        assert_eq!(press(&mut a, KeyCode::Enter), Action::Plan);
        assert_eq!(a.pending.len(), 1);
    }

    #[test]
    fn ajustes_con_opciones() {
        let mut a = app(true, true);
        press(&mut a, KeyCode::Left);
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Right);
        assert_eq!(a.views[a.selected().unwrap()].id, "search.taskbar_mode");
        // Espacio abre el selector parado en la opción vigente.
        press(&mut a, KeyCode::Char(' '));
        assert!(matches!(a.modal, Modal::Choice { sel: 1, .. }));
        press(&mut a, KeyCode::Up);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.pending[0].choice.as_deref(), Some("hidden"));
        assert!(a.pending[0].applied);
        // Volver a elegir la vigente desmarca.
        press(&mut a, KeyCode::Char(' '));
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Enter);
        assert!(a.pending.is_empty());
    }

    #[test]
    fn buscar_filtra_en_todas_las_categorias() {
        let mut a = app(true, true);
        press(&mut a, KeyCode::Char('/'));
        // Mientras se escribe, las letras son texto: la q no sale.
        type_text(&mut a, "izq");
        assert_eq!(a.visible().len(), 1);
        assert_eq!(a.views[a.selected().unwrap()].id, "taskbar.align_left");
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Char(' '));
        assert_eq!(a.pending[0].id, "taskbar.align_left");
        // Esc borra el filtro y vuelve a la categoría, sin perder lo marcado.
        press(&mut a, KeyCode::Esc);
        assert!(!a.searching());
        assert_eq!(a.pending.len(), 1);
        assert_eq!(press(&mut a, KeyCode::Char('q')), Action::Quit);
    }

    #[test]
    fn el_plan_se_confirma_o_se_cancela() {
        let mut a = app(true, true);
        press(&mut a, KeyCode::Char(' '));
        let change = |noop| PlannedChange {
            tweak_id: "privacy.advertising_id".into(),
            key: r"HKCU\Software\Prueba".into(),
            name: "Valor".into(),
            from: "0".into(),
            to: "1".into(),
            noop,
        };
        a.show_plan(Ok(vec![change(false)]));
        assert_eq!(a.changing().len(), 1);
        assert_eq!(press(&mut a, KeyCode::Esc), Action::None);
        assert!(matches!(a.modal, Modal::None));
        // Cancelar no pierde las marcas.
        assert_eq!(a.pending.len(), 1);
        a.show_plan(Ok(vec![change(false)]));
        assert_eq!(press(&mut a, KeyCode::Enter), Action::Apply);
        // Si nada cambia, Enter no aplica.
        a.show_plan(Ok(vec![change(true)]));
        assert!(a.changing().is_empty());
        assert_eq!(press(&mut a, KeyCode::Enter), Action::None);
        // Un plan con error tampoco.
        a.show_plan(Err("no se pudo".into()));
        assert_eq!(press(&mut a, KeyCode::Enter), Action::None);
    }

    #[test]
    fn despues_de_aplicar_se_limpian_las_marcas() {
        let mut a = app(true, true);
        press(&mut a, KeyCode::Char(' '));
        let report: ApplyReport = ApplyReport {
            entry_id: Some(1),
            results: vec![ItemResult { id: "privacy.advertising_id".into(), ok: true, error: None }],
            restart: Restart::Explorer,
            conflicts: vec![],
            warning: None,
        };
        let views = a.views.clone();
        a.applied(&report, views);
        assert!(a.pending.is_empty());
        let s = screen(&mut a, 100, 30);
        assert!(s.contains("✓ Desactivar ID de publicidad"), "{s}");
        assert!(s.contains("reiniciar el Explorador"), "{s}");
        // Cualquier tecla cierra el resultado.
        press(&mut a, KeyCode::Char('x'));
        assert!(matches!(a.modal, Modal::None));
    }

    #[test]
    fn deshacer_pide_confirmacion() {
        let mut a = app(true, true);
        assert_eq!(press(&mut a, KeyCode::Char('u')), Action::AskUndo);
        a.ask_undo(42, "✓ Algo".into());
        assert_eq!(press(&mut a, KeyCode::Esc), Action::None);
        a.ask_undo(42, "✓ Algo".into());
        assert_eq!(press(&mut a, KeyCode::Enter), Action::Undo(42));
    }

    #[test]
    fn dibuja_paneles_detalle_y_ayuda() {
        let mut a = app(false, true);
        let s = screen(&mut a, 100, 30);
        for expected in ["P-Rune", "Categorías", "Privacidad", "1/2", "● Desactivar ID de publicidad", "○ Telemetría al mínimo", "admin", "privacy.advertising_id", "Descripción de", "espacio", "q salir"] {
            assert!(s.contains(expected), "falta {expected:?} en:\n{s}");
        }
        press(&mut a, KeyCode::Char(' '));
        assert!(screen(&mut a, 100, 30).contains("- Desactivar ID de publicidad"));
        assert!(screen(&mut a, 100, 30).contains("revisar y aplicar (1)"));
        press(&mut a, KeyCode::Char('?'));
        assert!(screen(&mut a, 100, 30).contains("Teclas"));
    }

    #[test]
    fn dibuja_el_plan() {
        let mut a = app(true, true);
        press(&mut a, KeyCode::Char(' '));
        a.show_plan(Ok(vec![PlannedChange {
            tweak_id: "privacy.advertising_id".into(),
            key: r"HKCU\Software\Prueba".into(),
            name: "Valor".into(),
            from: "1".into(),
            to: "0".into(),
            noop: false,
        }]));
        let s = screen(&mut a, 100, 30);
        assert!(s.contains(r"HKCU\Software\Prueba\Valor: 1 → 0"), "{s}");
        assert!(s.contains("enter aplicar"), "{s}");
    }

    #[test]
    fn ventana_chica_no_rompe() {
        let mut a = app(true, true);
        assert!(screen(&mut a, 40, 8).contains("Agrandá"));
        // Y en el mínimo que sí se dibuja, tampoco.
        screen(&mut a, 60, 14);
        a.modal = Modal::Help;
        screen(&mut a, 60, 14);
    }
}
