//! The editor's document model: annotations layered over a captured image, the
//! tool state machine that turns pointer input into edits, hit-testing, and
//! snapshot-based undo/redo.
//!
//! All coordinates are image pixels with a top-left origin. Sizes are defined
//! in points and multiplied by the image's pixel density (`scale`) so that
//! annotations look the same on Retina and non-Retina captures.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Arrow,
    Rect,
    Ellipse,
    Line,
    Text,
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Arrow,
    Rect,
    Ellipse,
    Line,
    Text,
    Step,
}

impl Tool {
    pub const ALL: [Tool; 7] =
        [Tool::Select, Tool::Arrow, Tool::Rect, Tool::Ellipse, Tool::Line, Tool::Text, Tool::Step];

    fn shape(self) -> Option<Kind> {
        Some(match self {
            Tool::Arrow => Kind::Arrow,
            Tool::Rect => Kind::Rect,
            Tool::Ellipse => Kind::Ellipse,
            Tool::Line => Kind::Line,
            _ => return None,
        })
    }
}

/// The palette lives in the renderer; the model only stores an index.
pub const NUM_COLORS: usize = 6;
pub const NUM_SIZES: usize = 3;

const STROKE_PT: [f64; NUM_SIZES] = [3.0, 5.0, 8.0];
const FONT_PT: [f64; NUM_SIZES] = [16.0, 24.0, 36.0];
const STEP_PT: [f64; NUM_SIZES] = [12.0, 16.0, 22.0]; // marker radius
const HIT_PT: f64 = 6.0; // pointer tolerance
const MIN_PT: f64 = 4.0; // shapes shorter than this are discarded as stray clicks

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub const fn pt(x: f64, y: f64) -> Point {
    Point { x, y }
}

impl Point {
    fn add(self, q: Point) -> Point {
        pt(self.x + q.x, self.y + q.y)
    }
    fn sub(self, q: Point) -> Point {
        pt(self.x - q.x, self.y - q.y)
    }
    fn len(self) -> f64 {
        self.x.hypot(self.y)
    }
}

/// One annotation. For shapes a→b is the drag; for text a is the top-left
/// corner and b the bottom-right (from the measured size); for steps a is the
/// centre.
#[derive(Clone, Debug, PartialEq)]
pub struct Annot {
    pub id: u32,
    pub kind: Kind,
    pub a: Point,
    pub b: Point,
    pub color: usize,
    pub size: usize,
    pub text: String,
}

/// What the renderer draws: an annotation with metrics resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub a: Point,
    pub b: Point,
    pub color: usize,
    pub stroke: f64,    // line width, px
    pub font_size: f64, // text and step numbers, px
    pub radius: f64,    // step marker, px
    pub number: usize,  // step number
    pub text: String,
    pub selected: bool,
}

/// Asks the UI to do something the model can't: open a text field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effect {
    pub begin_text: bool,
    pub text_id: u32, // 0 = new text
    pub text_at: Point,
    pub text: String,
}

/// Returns the rendered size, in px, of text at the given font size.
pub type MeasureFn = Box<dyn Fn(&str, f64) -> (f64, f64)>;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum DragKind {
    #[default]
    None,
    Create,
    Move,
}

#[derive(Default)]
struct Drag {
    kind: DragKind,
    id: u32,
    start: Point,
    orig_a: Point,
    orig_b: Point,
    before: Option<Vec<Annot>>,
}

#[allow(dead_code)] // w/h and a few queries are used by tests and kept for callers
pub struct Doc {
    pub w: f64,
    pub h: f64,
    pub scale: f64, // px per point

    pub annots: Vec<Annot>,

    pub tool: Tool,
    pub color: usize,
    pub size: usize,
    pub selected: u32, // annotation id; 0 = none
    pub editing: u32,  // text annotation hidden while its field is open; 0 = none

    measure: MeasureFn,
    undo: Vec<Vec<Annot>>,
    redo: Vec<Vec<Annot>>,
    next_id: u32,
    dirty: bool,
    drag: Drag,
}

#[allow(dead_code)]
impl Doc {
    pub fn new(w: f64, h: f64, scale: f64, measure: Option<MeasureFn>) -> Doc {
        let measure = measure.unwrap_or_else(|| {
            Box::new(|s: &str, fs: f64| (0.6 * fs * s.chars().count() as f64, 1.2 * fs))
        });
        Doc {
            w,
            h,
            scale: if scale > 0.0 { scale } else { 1.0 },
            annots: Vec::new(),
            tool: Tool::Arrow,
            color: 0,
            size: 0,
            selected: 0,
            editing: 0,
            measure,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 0,
            dirty: false,
            drag: Drag::default(),
        }
    }

    /// Whether there are edits that haven't been copied or saved.
    pub fn dirty(&self) -> bool {
        self.dirty
    }
    /// Marks the edits unexported again, e.g. after a copy failed.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn stroke(&self, size: usize) -> f64 {
        STROKE_PT[size] * self.scale
    }
    pub fn font_size(&self, size: usize) -> f64 {
        FONT_PT[size] * self.scale
    }
    pub fn radius(&self, size: usize) -> f64 {
        STEP_PT[size] * self.scale
    }

    fn index(&self, id: u32) -> Option<usize> {
        if id == 0 { None } else { self.annots.iter().position(|a| a.id == id) }
    }

    fn find(&mut self, id: u32) -> Option<&mut Annot> {
        self.index(id).map(|i| &mut self.annots[i])
    }

    fn add(&mut self, mut a: Annot) -> u32 {
        self.next_id += 1;
        a.id = self.next_id;
        self.annots.push(a);
        self.next_id
    }

    /// Records the current state as an undo step. Call before mutating.
    fn checkpoint(&mut self) {
        let before = self.annots.clone();
        self.push(before);
    }

    fn push(&mut self, before: Vec<Annot>) {
        self.undo.push(before);
        self.redo.clear();
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.annots, prev));
            self.after_restore();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.annots, next));
            self.after_restore();
        }
    }

    fn after_restore(&mut self) {
        self.dirty = true;
        self.drag = Drag::default();
        if self.index(self.selected).is_none() {
            self.selected = 0;
        }
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.tool = t;
    }

    /// Changes the current colour and recolours the selection.
    pub fn set_color(&mut self, c: usize) {
        if c >= NUM_COLORS {
            return;
        }
        self.color = c;
        if let Some(i) = self.index(self.selected)
            && self.annots[i].color != c
        {
            self.checkpoint();
            self.annots[i].color = c;
        }
    }

    /// Changes the current size and resizes the selection.
    pub fn set_size(&mut self, s: usize) {
        if s >= NUM_SIZES {
            return;
        }
        self.size = s;
        if let Some(i) = self.index(self.selected)
            && self.annots[i].size != s
        {
            self.checkpoint();
            self.annots[i].size = s;
            if self.annots[i].kind == Kind::Text {
                self.fit_text(i);
            }
        }
    }

    pub fn deselect(&mut self) {
        self.selected = 0;
    }

    pub fn delete_selected(&mut self) {
        if let Some(i) = self.index(self.selected) {
            self.checkpoint();
            self.annots.remove(i);
            self.selected = 0;
        }
    }

    fn fit_text(&mut self, i: usize) {
        let fs = self.font_size(self.annots[i].size);
        let (w, h) = (self.measure)(&self.annots[i].text, fs);
        let a = &mut self.annots[i];
        a.b = pt(a.a.x + w, a.a.y + h);
    }

    fn new_annot(&self, kind: Kind, a: Point) -> Annot {
        Annot { id: 0, kind, a, b: a, color: self.color, size: self.size, text: String::new() }
    }

    /// Handles a mouse-down at p.
    pub fn press(&mut self, p: Point, clicks: usize) -> Effect {
        self.drag = Drag::default();
        let hit = self.hit_test(p).map(|i| (self.annots[i].id, self.annots[i].kind, self.annots[i].a, self.annots[i].b));

        match self.tool {
            Tool::Select => {
                let Some((id, kind, a, b)) = hit else {
                    self.selected = 0;
                    return Effect::default();
                };
                self.selected = id;
                if kind == Kind::Text && clicks >= 2 {
                    return self.begin_edit(id);
                }
                self.drag = Drag {
                    kind: DragKind::Move,
                    id,
                    start: p,
                    orig_a: a,
                    orig_b: b,
                    before: Some(self.annots.clone()),
                };
            }
            Tool::Text => {
                if let Some((id, Kind::Text, ..)) = hit {
                    self.selected = id;
                    return self.begin_edit(id);
                }
                return Effect { begin_text: true, text_at: p, ..Effect::default() };
            }
            Tool::Step => {
                let before = self.annots.clone();
                let id = self.add(self.new_annot(Kind::Step, p));
                self.push(before);
                self.selected = id;
                // Dragging right after placing positions the marker, within the same undo step.
                self.drag = Drag { kind: DragKind::Move, id, start: p, orig_a: p, orig_b: p, before: None };
            }
            tool => {
                let kind = tool.shape().expect("shape tool");
                let before = self.annots.clone();
                let id = self.add(self.new_annot(kind, p));
                self.drag = Drag { kind: DragKind::Create, id, start: p, before: Some(before), ..Drag::default() };
            }
        }
        Effect::default()
    }

    fn begin_edit(&mut self, id: u32) -> Effect {
        self.editing = id;
        let a = self.find(id).expect("annotation");
        Effect { begin_text: true, text_id: id, text_at: a.a, text: a.text.clone() }
    }

    /// Handles pointer movement with the button down.
    pub fn drag_to(&mut self, p: Point) {
        let (kind, start, oa, ob) = (self.drag.kind, self.drag.start, self.drag.orig_a, self.drag.orig_b);
        let Some(a) = self.find(self.drag.id) else { return };
        match kind {
            DragKind::Create => a.b = p,
            DragKind::Move => {
                let off = p.sub(start);
                a.a = oa.add(off);
                a.b = ob.add(off);
            }
            DragKind::None => {}
        }
    }

    /// Handles a mouse-up at p.
    pub fn release(&mut self, p: Point) {
        self.drag_to(p);
        let dr = std::mem::take(&mut self.drag);
        let min = MIN_PT * self.scale;
        let Some(i) = self.index(dr.id) else { return };
        let (a, b, id) = (self.annots[i].a, self.annots[i].b, self.annots[i].id);
        match dr.kind {
            DragKind::Create => {
                let before = dr.before.expect("create snapshot");
                if b.sub(a).len() < min {
                    self.annots = before; // stray click: no shape, no undo step
                    return;
                }
                self.push(before);
                self.selected = id;
            }
            DragKind::Move => {
                if let Some(before) = dr.before
                    && a != dr.orig_a
                {
                    self.push(before);
                }
            }
            DragKind::None => {}
        }
    }

    /// Finishes a text field opened by an Effect. id 0 creates new text at
    /// `at`; empty text deletes an existing annotation or creates nothing.
    pub fn commit_text(&mut self, id: u32, at: Point, text: &str) {
        self.editing = 0;
        let text = text.trim_end();
        if id == 0 {
            if text.trim().is_empty() {
                return;
            }
            self.checkpoint();
            let mut a = self.new_annot(Kind::Text, at);
            a.text = text.to_string();
            let nid = self.add(a);
            let i = self.index(nid).unwrap();
            self.fit_text(i);
            self.selected = nid;
            return;
        }
        let Some(i) = self.index(id) else { return };
        if self.annots[i].text == text {
            return;
        }
        if text.trim().is_empty() {
            self.selected = id;
            self.delete_selected();
            return;
        }
        self.checkpoint();
        self.annots[i].text = text.to_string();
        self.fit_text(i);
    }

    /// Closes a text field without changes.
    pub fn cancel_text(&mut self) {
        self.editing = 0;
    }

    /// The 1-based position of a step among the steps, so numbers close up
    /// automatically when one is deleted.
    pub fn step_number(&self, id: u32) -> usize {
        self.annots
            .iter()
            .filter(|a| a.kind == Kind::Step)
            .position(|a| a.id == id)
            .map_or(0, |n| n + 1)
    }

    /// The number the step tool would place next.
    pub fn next_step(&self) -> usize {
        1 + self.annots.iter().filter(|a| a.kind == Kind::Step).count()
    }

    /// The items to draw, bottom to top.
    pub fn render(&self) -> Vec<Item> {
        let mut n = 0;
        let mut items = Vec::with_capacity(self.annots.len());
        for a in &self.annots {
            if a.kind == Kind::Step {
                n += 1;
            }
            if a.id == self.editing {
                continue;
            }
            let radius = self.radius(a.size);
            items.push(Item {
                kind: a.kind,
                a: a.a,
                b: a.b,
                color: a.color,
                stroke: self.stroke(a.size),
                font_size: if a.kind == Kind::Step { radius * 1.1 } else { self.font_size(a.size) },
                radius,
                number: if a.kind == Kind::Step { n } else { 0 },
                text: a.text.clone(),
                selected: a.id == self.selected,
            });
        }
        items
    }

    /// Index of the topmost annotation under p.
    pub fn hit_test(&self, p: Point) -> Option<usize> {
        (0..self.annots.len()).rev().find(|&i| {
            let a = &self.annots[i];
            a.id != self.editing && self.hits(a, p)
        })
    }

    fn hits(&self, a: &Annot, p: Point) -> bool {
        let tol = HIT_PT * self.scale + self.stroke(a.size) / 2.0;
        match a.kind {
            Kind::Arrow | Kind::Line => seg_dist(p, a.a, a.b) <= tol,
            Kind::Rect => {
                let r = Rect::norm(a.a, a.b);
                r.inset(-tol).contains(p) && !r.inset(tol).contains(p)
            }
            Kind::Ellipse => {
                let r = Rect::norm(a.a, a.b);
                let (rx, ry) = (r.w() / 2.0, r.h() / 2.0);
                if rx < 1.0 || ry < 1.0 {
                    return seg_dist(p, a.a, a.b) <= tol;
                }
                let c = pt(r.x0 + rx, r.y0 + ry);
                let (dx, dy) = ((p.x - c.x) / rx, (p.y - c.y) / ry);
                (dx.hypot(dy) - 1.0).abs() * rx.min(ry) <= tol
            }
            Kind::Text => Rect::norm(a.a, a.b).inset(-tol / 2.0).contains(p),
            Kind::Step => p.sub(a.a).len() <= self.radius(a.size) + HIT_PT * self.scale,
        }
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Rect {
    fn norm(a: Point, b: Point) -> Rect {
        Rect { x0: a.x.min(b.x), y0: a.y.min(b.y), x1: a.x.max(b.x), y1: a.y.max(b.y) }
    }
    fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    fn inset(&self, n: f64) -> Rect {
        Rect { x0: self.x0 + n, y0: self.y0 + n, x1: self.x1 - n, y1: self.y1 - n }
    }
    fn contains(&self, p: Point) -> bool {
        self.x0 <= self.x1 && self.y0 <= self.y1 && p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }
}

fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let ab = b.sub(a);
    let l2 = ab.x * ab.x + ab.y * ab.y;
    if l2 == 0.0 {
        return p.sub(a).len();
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / l2).clamp(0.0, 1.0);
    p.sub(pt(a.x + t * ab.x, a.y + t * ab.y)).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_doc() -> Doc {
        Doc::new(1000.0, 800.0, 2.0, None)
    }

    fn draw(d: &mut Doc, t: Tool, a: Point, b: Point) {
        d.set_tool(t);
        d.press(a, 1);
        d.drag_to(pt((a.x + b.x) / 2.0, (a.y + b.y) / 2.0));
        d.release(b);
    }

    #[test]
    fn draw_shapes() {
        let mut d = new_doc();
        for t in [Tool::Arrow, Tool::Rect, Tool::Ellipse, Tool::Line] {
            draw(&mut d, t, pt(100.0, 100.0), pt(300.0, 250.0));
        }
        assert_eq!(d.annots.len(), 4);
        for (a, k) in d.annots.iter().zip([Kind::Arrow, Kind::Rect, Kind::Ellipse, Kind::Line]) {
            assert_eq!((a.kind, a.a, a.b), (k, pt(100.0, 100.0), pt(300.0, 250.0)));
        }
        assert_eq!(d.selected, d.annots[3].id, "newest shape should be selected");
        assert!(d.dirty());
    }

    #[test]
    fn stray_click_discarded() {
        let mut d = new_doc();
        draw(&mut d, Tool::Rect, pt(10.0, 10.0), pt(12.0, 11.0));
        assert!(d.annots.is_empty() && !d.can_undo() && !d.dirty());
    }

    #[test]
    fn undo_redo() {
        let mut d = new_doc();
        draw(&mut d, Tool::Arrow, pt(0.0, 0.0), pt(100.0, 100.0));
        draw(&mut d, Tool::Rect, pt(200.0, 200.0), pt(400.0, 400.0));
        d.undo();
        assert_eq!(d.annots.len(), 1);
        assert_eq!(d.annots[0].kind, Kind::Arrow);
        d.undo();
        assert!(d.annots.is_empty() && !d.can_undo());
        d.redo();
        d.redo();
        assert!(d.annots.len() == 2 && !d.can_redo());
        d.undo();
        draw(&mut d, Tool::Line, pt(0.0, 0.0), pt(50.0, 50.0));
        assert!(!d.can_redo(), "new edit should clear redo");
    }

    #[test]
    fn select_move_delete_recolor() {
        let mut d = new_doc();
        draw(&mut d, Tool::Rect, pt(100.0, 100.0), pt(300.0, 300.0));
        let id = d.annots[0].id;

        d.set_tool(Tool::Select);
        d.press(pt(500.0, 500.0), 1);
        d.release(pt(500.0, 500.0));
        assert_eq!(d.selected, 0, "click on empty space should deselect");

        d.press(pt(200.0, 200.0), 1);
        d.release(pt(200.0, 200.0));
        assert_eq!(d.selected, 0, "rect interior should not select");

        d.press(pt(100.0, 200.0), 1);
        d.drag_to(pt(120.0, 210.0));
        d.release(pt(150.0, 250.0));
        assert_eq!(d.selected, id, "border click should select");
        assert_eq!((d.annots[0].a, d.annots[0].b), (pt(150.0, 150.0), pt(350.0, 350.0)));
        d.undo();
        assert_eq!(d.annots[0].a, pt(100.0, 100.0));
        d.redo();

        d.selected = id;
        d.set_color(3);
        assert!(d.annots[0].color == 3 && d.color == 3);
        d.undo();
        assert_eq!(d.annots[0].color, 0);

        d.selected = id;
        d.delete_selected();
        assert!(d.annots.is_empty() && d.selected == 0);
        d.undo();
        assert_eq!(d.annots.len(), 1);
    }

    #[test]
    fn click_without_move_adds_no_undo() {
        let mut d = new_doc();
        draw(&mut d, Tool::Line, pt(0.0, 0.0), pt(200.0, 0.0));
        let n = d.undo.len();
        d.set_tool(Tool::Select);
        d.press(pt(100.0, 2.0), 1);
        d.release(pt(100.0, 2.0));
        assert_eq!(d.undo.len(), n);
    }

    #[test]
    fn steps_renumber() {
        let mut d = new_doc();
        d.set_tool(Tool::Step);
        for x in [100.0, 200.0, 300.0, 400.0] {
            d.press(pt(x, 100.0), 1);
            d.release(pt(x, 100.0));
        }
        let nums = |d: &Doc| d.render().iter().map(|i| i.number).collect::<Vec<_>>();
        assert_eq!(nums(&d), [1, 2, 3, 4]);
        d.selected = d.annots[1].id; // marker "2"
        d.delete_selected();
        assert_eq!(nums(&d), [1, 2, 3]);
        assert_eq!(d.next_step(), 4);
        assert_eq!(d.step_number(d.annots[2].id), 3);
        d.undo();
        assert_eq!(nums(&d), [1, 2, 3, 4]);
    }

    #[test]
    fn step_place_is_one_undo() {
        let mut d = new_doc();
        d.set_tool(Tool::Step);
        d.press(pt(10.0, 10.0), 1);
        d.drag_to(pt(50.0, 50.0));
        d.release(pt(60.0, 60.0));
        assert_eq!(d.undo.len(), 1);
        assert_eq!(d.annots[0].a, pt(60.0, 60.0));
        d.undo();
        assert!(d.annots.is_empty());
    }

    #[test]
    fn text() {
        let mut d = new_doc();
        d.set_tool(Tool::Text);
        let eff = d.press(pt(50.0, 60.0), 1);
        assert!(eff.begin_text && eff.text_id == 0 && eff.text_at == pt(50.0, 60.0));
        d.commit_text(0, eff.text_at, "hello");
        assert!(d.annots.len() == 1 && d.annots[0].text == "hello" && d.annots[0].b.x > 50.0);
        let id = d.annots[0].id;

        // Clicking existing text with the text tool edits it and hides it while editing.
        let eff = d.press(pt(55.0, 70.0), 1);
        assert!(eff.begin_text && eff.text_id == id && eff.text == "hello");
        assert!(d.render().is_empty());
        d.commit_text(id, eff.text_at, "hello world");
        assert!(d.annots[0].text == "hello world" && d.render().len() == 1);

        // Double-click with the select tool also edits.
        d.set_tool(Tool::Select);
        let eff = d.press(pt(55.0, 70.0), 2);
        assert!(eff.begin_text && eff.text_id == id);
        d.commit_text(id, eff.text_at, "  ");
        assert!(d.annots.is_empty(), "emptying text should delete it");

        d.set_tool(Tool::Text);
        let n = d.undo.len();
        let eff = d.press(pt(400.0, 400.0), 1);
        d.commit_text(0, eff.text_at, "");
        assert!(d.annots.is_empty() && d.undo.len() == n);
    }

    #[test]
    fn size_resizes_selection() {
        let mut d = new_doc();
        draw(&mut d, Tool::Arrow, pt(0.0, 0.0), pt(100.0, 100.0));
        d.set_size(2);
        let it = &d.render()[0];
        assert!(it.stroke == STROKE_PT[2] * 2.0 && it.selected);
    }

    #[test]
    fn hit_topmost() {
        let mut d = new_doc();
        draw(&mut d, Tool::Line, pt(0.0, 100.0), pt(200.0, 100.0));
        draw(&mut d, Tool::Line, pt(100.0, 0.0), pt(100.0, 200.0));
        assert_eq!(d.hit_test(pt(100.0, 100.0)), Some(1));
        assert_eq!(d.hit_test(pt(150.0, 150.0)), None);
    }

    #[test]
    fn mark_clean() {
        let mut d = new_doc();
        draw(&mut d, Tool::Arrow, pt(0.0, 0.0), pt(100.0, 100.0));
        d.mark_clean();
        assert!(!d.dirty());
        d.undo();
        assert!(d.dirty(), "undo after export should dirty the doc");
    }
}
