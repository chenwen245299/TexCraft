//! SyncTeX file reader: decompression, preamble, content tree and postamble.
//!
//! The content is turned into an arena of [`Node`]s (sheets, forms, boxes and the small
//! kern/glue/math/boundary records) linked as a first-child / next-sibling tree. While
//! building the tree we apply the same book-keeping the reference `synctex` tool applies
//! (visible hbox extents, mean line numbers, box boundary nodes, the "friend" registry used
//! for forward search), so that the search algorithms in `search.rs` answer exactly like
//! `synctex view` / `synctex edit`.

use std::collections::HashMap;
use std::io::Read;

use crate::Error;

/// "No node" sentinel for arena links.
pub(crate) const NONE: u32 = u32::MAX;
/// The C implementation works with `int` distances and uses `INT_MAX` as "infinitely far".
pub(crate) const INT_MAX: i64 = i32::MAX as i64;

/// Number of hash buckets of the reference implementation's friend lists. Nodes whose
/// tag/line change after registration stay in their original bucket, which makes them
/// invisible to forward search; we reproduce that quirk to match `synctex view`.
const FRIEND_BUCKETS: i64 = 1024;
/// Refuse to inflate more than this (gzip bombs).
const MAX_INFLATED: u64 = 1 << 30;
/// Maximum box nesting depth we build; deeper content is skipped (TeX never gets close).
const MAX_DEPTH: usize = 256;
/// Upper bound on nodes created by expanding form references.
const MAX_FORM_NODES: usize = 4_000_000;
/// Maximum nesting of form references inside forms.
const MAX_FORM_DEPTH: u32 = 8;

/// 1 bp in scaled points: 65536 * 72.27 / 72.
const SP_PER_BP: f64 = 65781.76;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Sheet,
    Form,
    VBox,
    HBox,
    VoidVBox,
    VoidHBox,
    Kern,
    Glue,
    Rule,
    Math,
    Boundary,
    /// Synthetic node at the start and end of every hbox.
    BoxBdry,
    /// Form reference (`f` record); replaced by a copy of the form after parsing.
    Ref,
}

impl Kind {
    /// Boxes in the sense of the reference implementation (`_synctex_node_is_box`).
    pub(crate) fn is_box(self) -> bool {
        matches!(
            self,
            Kind::VBox | Kind::HBox | Kind::VoidVBox | Kind::VoidHBox
        )
    }
    /// Kinds that carry width/height/depth fields (the "box" data model).
    fn has_box_data(self) -> bool {
        matches!(
            self,
            Kind::VBox | Kind::HBox | Kind::VoidVBox | Kind::VoidHBox | Kind::Rule
        )
    }
}

/// One record of the content tree. Coordinates are in SyncTeX units (scaled points times
/// `Unit`), y growing downwards.
#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub kind: Kind,
    pub tag: i32,
    pub line: i32,
    pub column: i32,
    pub h: i32,
    pub v: i32,
    pub width: i32,
    pub height: i32,
    pub depth: i32,
    /// Visible extent of hboxes (grown to contain their content).
    pub h_v: i32,
    pub v_v: i32,
    pub width_v: i32,
    pub height_v: i32,
    pub depth_v: i32,
    /// Weighted mean line of an hbox's content.
    pub mean_line: i32,
    pub weight: i32,
    pub parent: u32,
    pub first_child: u32,
    pub next: u32,
    /// Page of the enclosing sheet (sheet nodes: their own page; form content: 0).
    pub page: i32,
}

impl Node {
    fn new(kind: Kind) -> Self {
        Node {
            kind,
            tag: 0,
            line: 0,
            column: -1,
            h: 0,
            v: 0,
            width: 0,
            height: 0,
            depth: 0,
            h_v: 0,
            v_v: 0,
            width_v: 0,
            height_v: 0,
            depth_v: 0,
            mean_line: 0,
            weight: 0,
            parent: NONE,
            first_child: NONE,
            next: NONE,
            page: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Sheet {
    pub node: u32,
    pub page: i32,
    /// Hboxes of this sheet in the order they were closed. The reference implementation
    /// walks them from the last closed to the first.
    pub hboxes: Vec<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct Input {
    pub tag: i32,
    /// The name exactly as recorded (trailing spaces/CR trimmed).
    pub name: String,
}

/// Everything extracted from a SyncTeX file.
#[derive(Debug, Clone, Default)]
pub(crate) struct Data {
    pub nodes: Vec<Node>,
    pub sheets: Vec<Sheet>,
    pub inputs: Vec<Input>,
    /// Forward-search registry: (tag, line) -> nodes, in the reference friend-list order.
    pub friends: HashMap<(i32, i32), Vec<u32>>,
    /// Largest line recorded per input tag.
    pub max_line: HashMap<i32, i32>,
    pub magnification: i32,
    /// PDF points per SyncTeX unit (stored as `f32` like the reference tool).
    pub unit: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// Inflates gzip data (detected by its magic number) or borrows plain data.
pub(crate) fn inflate(bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, Error> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        let decoder = flate2::read::MultiGzDecoder::new(bytes);
        let result = decoder
            .take(MAX_INFLATED.saturating_add(1))
            .read_to_end(&mut out);
        if out.len() as u64 > MAX_INFLATED {
            return Err(Error::Format(
                "decompressed SyncTeX data is too large".into(),
            ));
        }
        match result {
            Ok(_) => Ok(std::borrow::Cow::Owned(out)),
            // A truncated or corrupt gzip stream: keep whatever was inflated.
            Err(e) if out.is_empty() => Err(Error::Format(format!("bad gzip data: {e}"))),
            Err(_) => Ok(std::borrow::Cow::Owned(out)),
        }
    } else {
        Ok(std::borrow::Cow::Borrowed(bytes))
    }
}

/// Parses (already inflated) SyncTeX text.
pub(crate) fn parse(text: &[u8]) -> Result<Data, Error> {
    let mut p = Parser::default();
    p.run(text)?;
    Ok(p.finish())
}

// ---------------------------------------------------------------------------------------
// Low-level field decoding, mirroring the reference scanner's leniency.

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn new(s: &'a [u8]) -> Self {
        Cursor { s, i: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// `strtol` at `start`: optional white space and sign, then decimal digits.
    /// Returns the value (saturated to `i32`) and the end index.
    fn strtol(&self, start: usize) -> Option<(i32, usize)> {
        let mut j = start;
        while matches!(self.s.get(j), Some(b' ' | b'\t' | b'\r' | 0x0b | 0x0c)) {
            j += 1;
        }
        let mut negative = false;
        if let Some(&c) = self.s.get(j)
            && (c == b'+' || c == b'-')
        {
            negative = c == b'-';
            j += 1;
        }
        let digits_start = j;
        let mut value: i64 = 0;
        while let Some(&c) = self.s.get(j) {
            if !c.is_ascii_digit() {
                break;
            }
            value = value
                .saturating_mul(10)
                .saturating_add(i64::from(c - b'0'))
                .min(i64::from(i32::MAX) + 1);
            j += 1;
        }
        if j == digits_start {
            return None;
        }
        let value = if negative { -value } else { value };
        let value = value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        Some((value, j))
    }

    /// An integer, optionally preceded by one `:` or `,` separator.
    fn int(&mut self) -> Option<i32> {
        let mut start = self.i;
        if matches!(self.s.get(start), Some(b':' | b',')) {
            start += 1;
        }
        let (value, end) = self.strtol(start)?;
        self.i = end;
        Some(value)
    }

    /// The optional `,column` of a link; `-1` when absent.
    fn column(&mut self) -> Option<i32> {
        if self.peek() == Some(b',') {
            let (value, end) = self.strtol(self.i + 1)?;
            self.i = end;
            Some(value)
        } else {
            Some(-1)
        }
    }

    /// A vertical coordinate: an integer, or `,=` meaning "same as the previous one".
    fn int_v(&mut self, last_v: &mut i32) -> Option<i32> {
        if let Some(v) = self.int() {
            *last_v = v;
            return Some(v);
        }
        if self
            .s
            .get(self.i..)
            .is_some_and(|rest| rest.starts_with(b",="))
        {
            self.i += 2;
            return Some(*last_v);
        }
        None
    }
}

/// `strtod` followed by an optional TeX unit, converted to scaled points.
fn parse_dimension(s: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(s).ok()?.trim_start();
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end += 1;
    }
    let mut seen_digit = false;
    while let Some(c) = bytes.get(end) {
        if c.is_ascii_digit() {
            seen_digit = true;
        } else if *c != b'.' {
            break;
        }
        end += 1;
    }
    if !seen_digit {
        return None;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut k = end + 1;
        if matches!(bytes.get(k), Some(b'+' | b'-')) {
            k += 1;
        }
        if bytes.get(k).is_some_and(u8::is_ascii_digit) {
            while bytes.get(k).is_some_and(u8::is_ascii_digit) {
                k += 1;
            }
            end = k;
        }
    }
    let value: f64 = text.get(..end)?.parse().ok()?;
    let unit = text.get(end..).unwrap_or("");
    let factor = [
        ("in", 72.27 * 65536.0),
        ("cm", 72.27 * 65536.0 / 2.54),
        ("mm", 72.27 * 65536.0 / 25.4),
        ("pt", 65536.0),
        ("bp", 72.27 / 72.0 * 65536.0),
        ("pc", 12.0 * 65536.0),
        ("sp", 1.0),
        ("dd", 1238.0 / 1157.0 * 65536.0),
        ("cc", 14856.0 / 1157.0 * 65536.0),
        ("nd", 685.0 / 642.0 * 65536.0),
        ("nc", 1370.0 / 107.0 * 65536.0),
    ]
    .iter()
    .find(|(name, _)| unit.starts_with(name))
    .map_or(1.0, |(_, f)| *f);
    let result = value * factor;
    result.is_finite().then_some(result)
}

fn parse_float(s: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(s).ok()?.trim();
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'e' | 'E')))
        .unwrap_or(text.len());
    let value: f64 = text.get(..end)?.parse().ok()?;
    value.is_finite().then_some(value)
}

fn strip_prefix<'a>(line: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    line.strip_prefix(prefix)
}

// ---------------------------------------------------------------------------------------
// The tree builder.

/// One open container (sheet, form or box) while parsing.
struct Level {
    node: u32,
    /// Last child appended (NONE if none yet).
    last: u32,
    /// Last child that is not a form reference.
    last_nonref: u32,
    /// Boundary (`x`) records at the start of the box whose link is not trusted: they take
    /// the link of the next record of this level (the reference tool's "x handles").
    pending: Vec<u32>,
    /// A kern followed by glue at the end of an hbox gets the link of the node before the
    /// kern (`last_k_prev`).
    last_k: u32,
    last_k_prev: u32,
    last_g: u32,
}

impl Level {
    fn new(node: u32) -> Self {
        Level {
            node,
            last: NONE,
            last_nonref: NONE,
            pending: Vec::new(),
            last_k: NONE,
            last_k_prev: NONE,
            last_g: NONE,
        }
    }
}

/// A sheet or a form being filled, with its stack of open boxes.
struct Context {
    stack: Vec<Level>,
    in_form: bool,
    /// Box records skipped because they were nested deeper than `MAX_DEPTH`.
    skipped_depth: u32,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Phase {
    Preamble,
    Main,
    Postamble,
    PostScriptum,
}

#[derive(Default)]
struct Parser {
    nodes: Vec<Node>,
    sheets: Vec<Sheet>,
    /// (form tag, form node), in definition order.
    forms: Vec<(i32, u32)>,
    inputs: Vec<Input>,
    /// (node, bucket) in registration order.
    registrations: Vec<(u32, u16)>,
    max_line: HashMap<i32, i32>,
    /// Form references found in sheets, in file order.
    sheet_refs: Vec<u32>,
    /// Open sheet/form contexts; more than one only when a form starts inside another.
    contexts: Vec<Context>,
    last_v: i32,
    pre_magnification: Option<i32>,
    pre_unit: Option<i32>,
    pre_x_offset: Option<i32>,
    pre_y_offset: Option<i32>,
    post_magnification: Option<f64>,
    post_x_offset: Option<f64>,
    post_y_offset: Option<f64>,
}

impl Parser {
    fn run(&mut self, text: &[u8]) -> Result<(), Error> {
        let mut lines = text.split(|&b| b == b'\n');
        // The header line may be preceded by junk, like the reference scanner allows.
        let mut found = false;
        for line in lines.by_ref() {
            if line.starts_with(b"SyncTeX Version:") {
                found = true;
                break;
            }
        }
        if !found {
            return Err(Error::Format("missing \"SyncTeX Version:\" header".into()));
        }
        self.last_v = -1;
        let mut phase = Phase::Preamble;
        for line in lines {
            phase = match phase {
                Phase::Preamble => self.preamble_line(line),
                Phase::Main => self.main_line(line),
                Phase::Postamble => self.postamble_line(line),
                Phase::PostScriptum => {
                    self.post_scriptum_line(line);
                    Phase::PostScriptum
                }
            };
        }
        // A truncated file: close whatever is still open so that the complete part of the
        // last sheet remains searchable.
        while !self.contexts.is_empty() {
            self.close_context();
        }
        Ok(())
    }

    fn preamble_line(&mut self, line: &[u8]) -> Phase {
        if self.input_line(line) {
            return Phase::Preamble;
        }
        let int_after =
            |prefix: &[u8]| strip_prefix(line, prefix).and_then(|r| Cursor::new(r).int());
        if line.starts_with(b"Content:") {
            return Phase::Main;
        }
        if line.starts_with(b"{") {
            // No "Content:" line; be lenient and treat this as content.
            return self.main_line(line);
        }
        if let Some(v) = int_after(b"Magnification:") {
            self.pre_magnification = Some(v);
        } else if let Some(v) = int_after(b"Unit:") {
            self.pre_unit = Some(v);
        } else if let Some(v) = int_after(b"X Offset:") {
            self.pre_x_offset = Some(v);
        } else if let Some(v) = int_after(b"Y Offset:") {
            self.pre_y_offset = Some(v);
        }
        Phase::Preamble
    }

    /// Handles an `Input:<tag>:<name>` record; returns whether `line` was one.
    fn input_line(&mut self, line: &[u8]) -> bool {
        let Some(rest) = strip_prefix(line, b"Input:") else {
            return false;
        };
        let mut cur = Cursor::new(rest);
        if let Some(tag) = cur.int() {
            // Step over the field separator, then the name runs to the end of the line.
            let name = rest.get(cur.i.saturating_add(1)..).unwrap_or(&[]);
            let mut name = String::from_utf8_lossy(name).into_owned();
            while name.ends_with(' ') || name.ends_with('\r') {
                name.pop();
            }
            if !name.is_empty() {
                self.inputs.push(Input { tag, name });
            }
        }
        true
    }

    /// Outside of sheets and forms.
    fn main_line(&mut self, line: &[u8]) -> Phase {
        if !self.contexts.is_empty() {
            return self.content_line(line);
        }
        match line.first() {
            Some(b'{') => {
                if let Some(page) = Cursor::new(line.get(1..).unwrap_or(&[])).int() {
                    let mut node = Node::new(Kind::Sheet);
                    node.page = page;
                    let id = self.push_node(node);
                    self.sheets.push(Sheet {
                        node: id,
                        page,
                        hboxes: Vec::new(),
                    });
                    self.contexts.push(Context {
                        stack: vec![Level::new(id)],
                        in_form: false,
                        skipped_depth: 0,
                    });
                }
            }
            Some(b'<') => self.begin_form(line),
            Some(b'P') if line.starts_with(b"Postamble:") => return Phase::Postamble,
            Some(b'I') => {
                self.input_line(line);
            }
            _ => {}
        }
        Phase::Main
    }

    fn begin_form(&mut self, line: &[u8]) {
        if let Some(tag) = Cursor::new(line.get(1..).unwrap_or(&[])).int() {
            let mut node = Node::new(Kind::Form);
            node.tag = tag;
            let id = self.push_node(node);
            self.forms.push((tag, id));
            self.contexts.push(Context {
                stack: vec![Level::new(id)],
                in_form: true,
                skipped_depth: 0,
            });
        }
    }

    fn postamble_line(&mut self, line: &[u8]) -> Phase {
        if line.starts_with(b"Post scriptum:") {
            Phase::PostScriptum
        } else {
            Phase::Postamble
        }
    }

    fn post_scriptum_line(&mut self, line: &[u8]) {
        if let Some(rest) = strip_prefix(line, b"Magnification:") {
            if let Some(m) = parse_float(rest).filter(|m| *m > 0.0) {
                self.post_magnification = Some(m);
            }
        } else if let Some(rest) = strip_prefix(line, b"X Offset:") {
            if let Some(x) = parse_dimension(rest) {
                self.post_x_offset = Some(x);
            }
        } else if let Some(rest) = strip_prefix(line, b"Y Offset:")
            && let Some(y) = parse_dimension(rest)
        {
            self.post_y_offset = Some(y);
        }
    }

    fn push_node(&mut self, node: Node) -> u32 {
        let id = u32::try_from(self.nodes.len()).unwrap_or(NONE);
        if id != NONE {
            self.nodes.push(node);
        }
        id
    }

    fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.get(id as usize)
    }

    fn node_mut(&mut self, id: u32) -> Option<&mut Node> {
        self.nodes.get_mut(id as usize)
    }

    fn kind(&self, id: u32) -> Option<Kind> {
        self.node(id).map(|n| n.kind)
    }

    /// Adds `node` to the friend registry under its current tag and line.
    fn register(&mut self, id: u32) {
        let Some(n) = self.node(id) else { return };
        let key = i64::from(n.tag) + i64::from(n.line);
        if key >= 0 {
            self.registrations.push((id, (key % FRIEND_BUCKETS) as u16));
        }
    }

    fn note_line(&mut self, id: u32) {
        if let Some(n) = self.node(id) {
            let (tag, line) = (n.tag, n.line);
            let entry = self.max_line.entry(tag).or_insert(0);
            if line > *entry {
                *entry = line;
            }
        }
    }

    fn copy_link(&mut self, to: u32, from: u32) {
        if let Some(f) = self.node(from) {
            let (tag, line, column) = (f.tag, f.line, f.column);
            if let Some(t) = self.node_mut(to) {
                t.tag = tag;
                t.line = line;
                t.column = column;
            }
        }
    }

    /// Appends `child` to the innermost open container of the current context.
    fn append(&mut self, child: u32) {
        let Some(level) = self.contexts.last_mut().and_then(|c| c.stack.last_mut()) else {
            return;
        };
        let (parent, prev) = (level.node, level.last);
        level.last = child;
        let page = self.node(parent).map_or(0, |p| p.page);
        if let Some(c) = self.node_mut(child) {
            c.parent = parent;
            c.page = page;
        }
        if prev == NONE {
            if let Some(p) = self.node_mut(parent) {
                p.first_child = child;
            }
        } else if let Some(p) = self.node_mut(prev) {
            p.next = child;
        }
        if self.kind(child) != Some(Kind::Ref)
            && let Some(level) = self.contexts.last_mut().and_then(|c| c.stack.last_mut())
        {
            level.last_nonref = child;
        }
    }

    fn in_form(&self) -> bool {
        self.contexts.last().is_some_and(|c| c.in_form)
    }

    /// The pending boundaries of the current level take the link of `model` and become
    /// searchable (newest first, as in the reference implementation).
    fn resolve_pending(&mut self, model: u32) {
        let in_form = self.in_form();
        let pending = match self.contexts.last_mut().and_then(|c| c.stack.last_mut()) {
            Some(level) => std::mem::take(&mut level.pending),
            None => return,
        };
        for &x in pending.iter().rev() {
            self.copy_link(x, model);
            if !in_form {
                self.register(x);
            }
        }
    }

    fn reset_kern_glue(&mut self) {
        if let Some(level) = self.contexts.last_mut().and_then(|c| c.stack.last_mut()) {
            level.last_k = NONE;
            level.last_g = NONE;
        }
    }

    fn current_container(&self) -> Option<u32> {
        self.contexts
            .last()
            .and_then(|c| c.stack.last())
            .map(|l| l.node)
    }

    /// Grows the visible extent of hbox `parent` to contain the point (`h`, `v`).
    fn hbox_contain_point(&mut self, parent: u32, h: i64, v: i64) {
        let Some(p) = self.node_mut(parent) else {
            return;
        };
        if p.kind != Kind::HBox {
            return;
        }
        let n = i64::from(p.width_v);
        if n < 0 {
            let max = i64::from(p.h_v);
            let min = max + n;
            if h < min {
                p.width_v = sat(h - max);
            } else if h > max {
                p.h_v = sat(h);
                p.width_v = sat(min - h);
            }
        } else {
            let min = i64::from(p.h_v);
            let max = min + n;
            if h < min {
                p.h_v = sat(h);
                p.width_v = sat(max - h);
            } else if h > max {
                p.width_v = sat(h - min);
            }
        }
        let n = i64::from(p.v_v);
        let min = n - i64::from(p.height_v);
        let max = n + i64::from(p.depth_v);
        if v < min {
            p.height_v = sat(n - v);
        } else if v > max {
            p.depth_v = sat(v - n);
        }
    }

    /// Grows the visible extent of hbox `parent` to contain the box
    /// `[min_h, max_h] x [min_v, max_v]`.
    fn hbox_contain_box(&mut self, parent: u32, b: [i64; 4]) {
        let [min_h, max_h, min_v, max_v] = b;
        let Some(p) = self.node_mut(parent) else {
            return;
        };
        if p.kind != Kind::HBox {
            return;
        }
        let n = i64::from(p.width_v);
        if n < 0 {
            let max = i64::from(p.h_v);
            let min = max + n;
            if min_h < min {
                p.width_v = sat(min_h - max);
            } else if max_h > max {
                p.h_v = sat(max_h);
                p.width_v = sat(min - max_h);
            }
        } else {
            let min = i64::from(p.h_v);
            let max = min + n;
            if min_h < min {
                p.h_v = sat(min_h);
                p.width_v = sat(max - min_h);
            } else if max_h > max {
                p.width_v = sat(max_h - min);
            }
        }
        let n = i64::from(p.v_v);
        let min = n - i64::from(p.height_v);
        let max = n + i64::from(p.depth_v);
        if min_v < min {
            p.height_v = sat(n - min_v);
        } else if max_v > max {
            p.depth_v = sat(max_v - n);
        }
    }

    /// Parses the link and geometry of a record (after its leading character).
    fn record(&mut self, kind: Kind, body: &[u8]) -> Option<Node> {
        let mut cur = Cursor::new(body);
        let mut node = Node::new(kind);
        node.tag = cur.int()?;
        node.line = cur.int()?;
        node.column = cur.column()?;
        node.h = cur.int()?;
        node.v = cur.int_v(&mut self.last_v)?;
        if kind.has_box_data() || kind == Kind::Kern {
            node.width = cur.int()?;
        }
        if kind.has_box_data() {
            node.height = cur.int()?;
            node.depth = cur.int()?;
        }
        Some(node)
    }

    /// A line inside a sheet or a form.
    fn content_line(&mut self, line: &[u8]) -> Phase {
        let Some(&first) = line.first() else {
            return Phase::Main;
        };
        let body = line.get(1..).unwrap_or(&[]);
        // Records nested deeper than MAX_DEPTH are skipped wholesale.
        if let Some(ctx) = self.contexts.last_mut()
            && ctx.skipped_depth > 0
        {
            match first {
                b'[' | b'(' => ctx.skipped_depth = ctx.skipped_depth.saturating_add(1),
                b']' | b')' => ctx.skipped_depth -= 1,
                _ => {}
            }
            return Phase::Main;
        }
        match first {
            b'[' | b'(' => self.open_box(
                if first == b'[' {
                    Kind::VBox
                } else {
                    Kind::HBox
                },
                body,
            ),
            b']' => {
                if self.kind(self.current_container().unwrap_or(NONE)) == Some(Kind::VBox) {
                    self.close_vbox();
                } else {
                    self.reset_kern_glue();
                }
            }
            b')' => {
                if self.kind(self.current_container().unwrap_or(NONE)) == Some(Kind::HBox) {
                    self.close_hbox();
                } else {
                    self.reset_kern_glue();
                }
            }
            b'v' | b'h' | b'k' | b'g' | b'r' | b'$' | b'x' => {
                let kind = match first {
                    b'v' => Kind::VoidVBox,
                    b'h' => Kind::VoidHBox,
                    b'k' => Kind::Kern,
                    b'g' => Kind::Glue,
                    b'r' => Kind::Rule,
                    b'$' => Kind::Math,
                    _ => Kind::Boundary,
                };
                match self.record(kind, body) {
                    Some(node) => self.leaf(node),
                    None => self.reset_kern_glue(),
                }
            }
            b'f' => {
                let mut cur = Cursor::new(body);
                let parsed = (|| {
                    let tag = cur.int()?;
                    let h = cur.int()?;
                    let v = cur.int_v(&mut self.last_v)?;
                    Some((tag, h, v))
                })();
                if let Some((tag, h, v)) = parsed {
                    let mut node = Node::new(Kind::Ref);
                    node.tag = tag;
                    node.column = 0;
                    node.h = h;
                    node.v = v;
                    let id = self.push_node(node);
                    if id != NONE {
                        self.append(id);
                        if !self.in_form() {
                            self.sheet_refs.push(id);
                        }
                    }
                }
                self.reset_kern_glue();
            }
            b'}' => {
                let at_top = self
                    .contexts
                    .last()
                    .is_some_and(|c| !c.in_form && c.stack.len() == 1);
                if at_top {
                    self.close_context();
                } else {
                    self.reset_kern_glue();
                }
            }
            b'<' => self.begin_form(line),
            b'>' => {
                let at_top = self
                    .contexts
                    .last()
                    .is_some_and(|c| c.in_form && c.stack.len() == 1);
                if at_top {
                    self.close_context();
                } else {
                    self.reset_kern_glue();
                }
            }
            b'I' => {
                self.input_line(line);
            }
            b'P' if line.starts_with(b"Postamble:") => {
                while !self.contexts.is_empty() {
                    self.close_context();
                }
                return Phase::Postamble;
            }
            // Anchors (`!`), characters (`c`), comments and anything unknown.
            _ => self.reset_kern_glue(),
        }
        Phase::Main
    }

    fn open_box(&mut self, kind: Kind, body: &[u8]) {
        let depth_exceeded = self
            .contexts
            .last()
            .is_some_and(|c| c.stack.len() > MAX_DEPTH);
        if depth_exceeded {
            if let Some(ctx) = self.contexts.last_mut() {
                ctx.skipped_depth = 1;
            }
            return;
        }
        let Some(mut node) = self.record(kind, body) else {
            self.reset_kern_glue();
            return;
        };
        if kind == Kind::HBox {
            node.h_v = node.h;
            node.v_v = node.v;
            node.width_v = node.width;
            node.height_v = node.height;
            node.depth_v = node.depth;
        }
        let id = self.push_node(node);
        if id == NONE {
            return;
        }
        self.append(id);
        if let Some(ctx) = self.contexts.last_mut() {
            ctx.stack.push(Level::new(id));
        }
        self.note_line(id);
        if kind == Kind::HBox {
            // Leading box boundary: same link and origin as the box.
            let mut bdry = Node::new(Kind::BoxBdry);
            if let Some(b) = self.node(id) {
                bdry.tag = b.tag;
                bdry.line = b.line;
                bdry.column = b.column;
                bdry.h = b.h;
                bdry.v = b.v;
            }
            let bid = self.push_node(bdry);
            if bid != NONE {
                self.append(bid);
                if !self.in_form() {
                    self.register(bid);
                }
            }
        }
    }

    /// Kern, glue, rule, math, boundary and void box records.
    fn leaf(&mut self, node: Node) {
        let kind = node.kind;
        let (h, v) = (i64::from(node.h), i64::from(node.v));
        let (w, ht, dp) = (
            i64::from(node.width),
            i64::from(node.height),
            i64::from(node.depth),
        );
        let id = self.push_node(node);
        if id == NONE {
            return;
        }
        let in_form = self.in_form();
        let Some(level) = self.contexts.last().and_then(|c| c.stack.last()) else {
            return;
        };
        let (parent, prev, has_pending) = (level.node, level.last, !level.pending.is_empty());
        self.append(id);
        self.note_line(id);
        match kind {
            Kind::Boundary => {
                if self.kind(prev) == Some(Kind::BoxBdry) || has_pending {
                    if let Some(level) = self.contexts.last_mut().and_then(|c| c.stack.last_mut()) {
                        level.pending.push(id);
                    }
                } else if !in_form {
                    self.register(id);
                }
                self.hbox_contain_point(parent, h, v);
                self.reset_kern_glue();
            }
            Kind::VoidVBox | Kind::VoidHBox => {
                self.resolve_pending(id);
                if kind == Kind::VoidHBox {
                    let (min_h, max_h) = if w < 0 { (h + w, h) } else { (h, h + w) };
                    self.hbox_contain_box(parent, [min_h, max_h, v - ht, v + dp]);
                }
                self.reset_kern_glue();
            }
            Kind::Kern | Kind::Glue | Kind::Rule | Kind::Math => {
                if !in_form {
                    self.register(id);
                }
                self.resolve_pending(id);
                match kind {
                    Kind::Kern => {
                        let (min_h, max_h) = if w > 0 { (h - w, h) } else { (h, h - w) };
                        self.hbox_contain_box(parent, [min_h, max_h, v, v]);
                    }
                    Kind::Glue | Kind::Math => self.hbox_contain_point(parent, h, v),
                    _ => {}
                }
                if let Some(level) = self.contexts.last_mut().and_then(|c| c.stack.last_mut()) {
                    match kind {
                        Kind::Kern => {
                            level.last_k = id;
                            level.last_k_prev = prev;
                            level.last_g = NONE;
                        }
                        Kind::Glue if level.last_k != NONE => level.last_g = id,
                        _ => {
                            level.last_k = NONE;
                            level.last_g = NONE;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Pops the innermost box level; its leftover pending boundaries keep their own link,
    /// the parent's pending boundaries take the link of the closed box.
    fn pop_level(&mut self, closed: u32) {
        let in_form = self.in_form();
        let Some(level) = self.contexts.last_mut().and_then(|c| c.stack.pop()) else {
            return;
        };
        if !in_form {
            for &x in level.pending.iter().rev() {
                self.register(x);
            }
        }
        self.resolve_pending(closed);
        self.reset_kern_glue();
    }

    fn close_vbox(&mut self) {
        let Some(id) = self.current_container() else {
            return;
        };
        if self.node(id).is_some_and(|n| n.first_child == NONE) && !self.in_form() {
            self.register(id);
        }
        self.pop_level(id);
    }

    fn close_hbox(&mut self) {
        let Some(level) = self.contexts.last().and_then(|c| c.stack.last()) else {
            return;
        };
        let (id, last, last_nonref) = (level.node, level.last, level.last_nonref);
        let (last_k, last_k_prev, last_g) = (level.last_k, level.last_k_prev, level.last_g);
        let in_form = self.in_form();
        if !in_form && let Some(sheet) = self.sheets.last_mut() {
            sheet.hboxes.push(id);
        }
        // Mean line of the content, ignoring the leading box boundary.
        let first = self.node(id).map_or(NONE, |n| n.first_child);
        let second = self.node(first).map_or(NONE, |n| n.next);
        if second != NONE {
            let second_line = self.node(second).map_or(0, |n| n.line);
            if let Some(f) = self.node_mut(first) {
                f.line = second_line;
            }
            let mut weight: i64 = 0;
            let mut sum: i64 = 0;
            let mut cur = second;
            while let Some(n) = self.node(cur) {
                if n.kind == Kind::HBox {
                    if n.weight != 0 {
                        weight += i64::from(n.weight);
                        sum += i64::from(n.mean_line) * i64::from(n.weight);
                    } else {
                        weight += 1;
                        sum += i64::from(n.mean_line);
                    }
                } else {
                    weight += 1;
                    sum += i64::from(n.line);
                }
                cur = n.next;
            }
            if let Some(b) = self.node_mut(id) {
                b.mean_line = sat((sum + weight / 2) / weight.max(1));
                b.weight = sat(weight);
            }
        } else if let Some(b) = self.node_mut(id) {
            b.mean_line = b.line;
            b.weight = 1;
        }
        // Trailing box boundary: link of the last child, at the right end of the box.
        let (h_v, v_v, width_v) = self
            .node(id)
            .map_or((0, 0, 0), |b| (b.h_v, b.v_v, b.width_v));
        let mut bdry = Node::new(Kind::BoxBdry);
        bdry.h = sat(i64::from(h_v) + i64::from(width_v));
        bdry.v = v_v;
        let model = if last_nonref != NONE {
            last_nonref
        } else {
            last
        };
        let bid = self.push_node(bdry);
        if bid != NONE {
            self.copy_link(bid, model);
            self.append(bid);
        }
        if let Some(f) = self.node_mut(first) {
            f.h = h_v;
            f.v = v_v;
        }
        if last_k != NONE && last_g != NONE && last_k_prev != NONE {
            self.copy_link(last_k, last_k_prev);
            self.copy_link(last_g, last_k_prev);
        }
        self.pop_level(id);
        if let (Some(parent), Some(b)) = (self.current_container(), self.node(id)) {
            let n = i64::from(b.width_v);
            let (min_h, max_h) = if n < 0 {
                (i64::from(b.h_v) + n, i64::from(b.h_v))
            } else {
                (i64::from(b.h_v), i64::from(b.h_v) + n)
            };
            let v = i64::from(b.v_v);
            let r = [
                min_h,
                max_h,
                v - i64::from(b.height_v),
                v + i64::from(b.depth_v),
            ];
            self.hbox_contain_box(parent, r);
        }
    }

    /// Closes the current sheet or form (and any box left open in a truncated file).
    fn close_context(&mut self) {
        loop {
            let Some(ctx) = self.contexts.last() else {
                return;
            };
            if ctx.stack.len() <= 1 {
                break;
            }
            let Some(id) = self.current_container() else {
                break;
            };
            match self.kind(id) {
                Some(Kind::HBox) => self.close_hbox(),
                Some(Kind::VBox) => self.close_vbox(),
                _ => {
                    if let Some(ctx) = self.contexts.last_mut() {
                        ctx.stack.pop();
                    }
                }
            }
        }
        self.contexts.pop();
    }

    // -----------------------------------------------------------------------------------
    // Post-processing.

    /// Replaces form references in sheets by copies of the form content (the reference
    /// implementation uses proxies; copies behave the same for searching).
    fn expand_refs(&mut self) {
        let refs = std::mem::take(&mut self.sheet_refs);
        let budget = self.nodes.len().saturating_mul(4).min(MAX_FORM_NODES);
        let mut created = 0usize;
        for r in refs {
            self.expand_ref(r, 0, &mut created, budget);
        }
    }

    fn form_content(&self, tag: i32) -> u32 {
        self.forms
            .iter()
            .find(|(t, _)| *t == tag)
            .and_then(|(_, f)| self.node(*f))
            .map_or(NONE, |f| f.first_child)
    }

    /// Turns the ref node `r` (already linked in its parent) into a copy of its form's
    /// content root, then copies the content's children below it. Unresolvable refs are
    /// unlinked, like the reference implementation does.
    fn expand_ref(&mut self, r: u32, form_depth: u32, created: &mut usize, budget: usize) {
        let Some(rn) = self.node(r).cloned() else {
            return;
        };
        let target = self.form_content(rn.tag);
        let Some(t) = self.node(target).cloned() else {
            self.unlink(r);
            return;
        };
        if form_depth > MAX_FORM_DEPTH || *created >= budget || t.kind == Kind::Ref {
            self.unlink(r);
            return;
        }
        let (dh, dv) = (i64::from(rn.h), i64::from(rn.v));
        let mut copy = t.clone();
        shift(&mut copy, dh, dv);
        copy.parent = rn.parent;
        copy.next = rn.next;
        copy.first_child = NONE;
        copy.page = rn.page;
        if let Some(slot) = self.node_mut(r) {
            *slot = copy;
        }
        self.after_copy(r);
        let mut tree_depth = 0usize;
        let mut p = r;
        while let Some(n) = self.node(p) {
            p = n.parent;
            tree_depth += 1;
        }
        let copy = Copy { dh, dv, form_depth };
        self.copy_children(target, r, &copy, tree_depth, created, budget);
    }

    fn after_copy(&mut self, id: u32) {
        self.register(id);
        if self.kind(id) == Some(Kind::HBox) {
            let page = self.node(id).map_or(0, |n| n.page);
            if let Some(sheet) = self.sheets.iter_mut().rev().find(|s| s.page == page) {
                sheet.hboxes.push(id);
            }
        }
    }

    /// Copies the children of `src` below `dst`, shifted by the form reference's offset.
    /// `tree_depth` is the depth of `dst` in its sheet; copies never nest deeper than
    /// `MAX_DEPTH` (plus a little), so the recursive searches stay bounded.
    fn copy_children(
        &mut self,
        src: u32,
        dst: u32,
        copy: &Copy,
        tree_depth: usize,
        created: &mut usize,
        budget: usize,
    ) {
        if tree_depth > MAX_DEPTH + 8 {
            return;
        }
        let page = self.node(dst).map_or(0, |n| n.page);
        let mut prev = NONE;
        let mut cur = self.node(src).map_or(NONE, |n| n.first_child);
        while let Some(n) = self.node(cur).cloned() {
            if *created >= budget {
                return;
            }
            *created += 1;
            let mut node = n.clone();
            shift(&mut node, copy.dh, copy.dv);
            node.parent = dst;
            node.next = NONE;
            node.first_child = NONE;
            node.page = page;
            let id = self.push_node(node);
            if id == NONE {
                return;
            }
            if prev == NONE {
                if let Some(d) = self.node_mut(dst) {
                    d.first_child = id;
                }
            } else if let Some(p) = self.node_mut(prev) {
                p.next = id;
            }
            prev = id;
            if n.kind == Kind::Ref {
                // A form used inside a form: expand it in place (offsets accumulate).
                self.expand_ref(id, copy.form_depth + 1, created, budget);
                // `expand_ref` may have unlinked the node; find the real last child.
                prev = self.last_child(dst);
            } else {
                self.after_copy(id);
                self.copy_children(cur, id, copy, tree_depth + 1, created, budget);
            }
            cur = n.next;
        }
    }

    fn last_child(&self, parent: u32) -> u32 {
        let mut cur = self.node(parent).map_or(NONE, |n| n.first_child);
        let mut last = NONE;
        while let Some(n) = self.node(cur) {
            last = cur;
            cur = n.next;
        }
        last
    }

    fn unlink(&mut self, id: u32) {
        let Some(n) = self.node(id) else { return };
        let (parent, next) = (n.parent, n.next);
        let first = self.node(parent).map_or(NONE, |p| p.first_child);
        if first == id {
            if let Some(p) = self.node_mut(parent) {
                p.first_child = next;
            }
        } else {
            let mut cur = first;
            while let Some(c) = self.node(cur) {
                if c.next == id {
                    if let Some(c) = self.node_mut(cur) {
                        c.next = next;
                    }
                    break;
                }
                cur = c.next;
            }
        }
        if let Some(n) = self.node_mut(id) {
            n.parent = NONE;
            n.next = NONE;
        }
    }

    fn finish(mut self) -> Data {
        self.expand_refs();
        // The friend lists are LIFO; a node is found under its current link only if that
        // link hashes to the bucket it was registered in.
        let mut friends: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for &(id, bucket) in self.registrations.iter().rev() {
            let Some(n) = self.nodes.get(id as usize) else {
                continue;
            };
            let key = i64::from(n.tag) + i64::from(n.line);
            if key >= 0 && key % FRIEND_BUCKETS == i64::from(bucket) {
                friends.entry((n.tag, n.line)).or_default().push(id);
            }
        }
        // Units, as the reference scanner computes them.
        let pre_unit = self.pre_unit.filter(|u| *u > 0).unwrap_or(1);
        let magnification = self.pre_magnification.filter(|m| *m > 0).unwrap_or(1000);
        let base = f64::from(pre_unit) / SP_PER_BP;
        let mut unit = base as f32;
        if let Some(m) = self.post_magnification {
            unit = (m * base) as f32;
        }
        let unit = (f64::from(unit) * (f64::from(magnification) / 1000.0)) as f32;
        let (x_offset, y_offset) = match self.post_x_offset {
            Some(x) => (
                (x / SP_PER_BP) as f32,
                self.post_y_offset.map_or_else(
                    || f64::from(self.pre_y_offset.unwrap_or(0)) * base,
                    |y| y / SP_PER_BP,
                ) as f32,
            ),
            None => (
                (f64::from(self.pre_x_offset.unwrap_or(0)) * base) as f32,
                (f64::from(self.pre_y_offset.unwrap_or(0)) * base) as f32,
            ),
        };
        Data {
            nodes: self.nodes,
            sheets: self.sheets,
            inputs: self.inputs,
            friends,
            max_line: self.max_line,
            magnification,
            unit,
            x_offset,
            y_offset,
        }
    }
}

/// Offset and nesting of a form reference being expanded.
struct Copy {
    dh: i64,
    dv: i64,
    form_depth: u32,
}

fn shift(n: &mut Node, dh: i64, dv: i64) {
    n.h = sat(i64::from(n.h) + dh);
    n.v = sat(i64::from(n.v) + dv);
    if n.kind == Kind::HBox {
        n.h_v = sat(i64::from(n.h_v) + dh);
        n.v_v = sat(i64::from(n.v_v) + dv);
    }
}

/// Saturating conversion back to the `i32` storage type.
pub(crate) fn sat(x: i64) -> i32 {
    x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_decodes_links_and_points() {
        let mut last_v = -1;
        let mut c = Cursor::new(b"1,22:4736286,4736286:26607616,44616926,0");
        assert_eq!(c.int(), Some(1));
        assert_eq!(c.int(), Some(22));
        assert_eq!(c.column(), Some(-1));
        assert_eq!(c.int(), Some(4736286));
        assert_eq!(c.int_v(&mut last_v), Some(4736286));
        assert_eq!(last_v, 4736286);
        assert_eq!(c.int(), Some(26607616));
        let mut c = Cursor::new(b"3,4,5:-10,=");
        assert_eq!(c.int(), Some(3));
        assert_eq!(c.int(), Some(4));
        assert_eq!(c.column(), Some(5));
        assert_eq!(c.int(), Some(-10));
        assert_eq!(c.int_v(&mut last_v), Some(4736286));
        assert_eq!(Cursor::new(b"99999999999999999999").int(), Some(i32::MAX));
        assert_eq!(Cursor::new(b":").int(), None);
    }

    #[test]
    fn dimensions() {
        assert_eq!(parse_dimension(b"1pt"), Some(65536.0));
        assert_eq!(parse_dimension(b"-1in"), Some(-72.27 * 65536.0));
        assert_eq!(parse_dimension(b"12"), Some(12.0));
        assert_eq!(parse_dimension(b"x"), None);
        assert_eq!(parse_float(b"1.5"), Some(1.5));
    }
}
