//! Forward (source → PDF) and inverse (PDF → source) search over the parsed tree.
//!
//! The heuristics follow the published behaviour of the reference `synctex` tool so that the
//! answers agree with `synctex view` / `synctex edit`:
//!
//! * forward search looks up the records registered for (tag, line), trying the lines
//!   `line, line+1, line-1, line+2, …` when a line has no records, keeps one record per
//!   enclosing hbox ordered by how many records of that line the hbox holds, and reports the
//!   largest enclosing hbox whose mean line stays within one line of the record's box;
//! * inverse search finds the smallest hbox containing the point, descends to the deepest
//!   container, and picks the closest children to the left/right of the point (falling back
//!   to the closest node of the page when no hbox contains the point).

use std::collections::{HashMap, HashSet};

use crate::parser::{Data, INT_MAX, Kind, NONE, Node};

/// More matches than this for one (tag, line) only happen with pathological files; the
/// rest are ignored to keep forward search fast.
const MAX_MATCHES: usize = 4096;

/// A forward-search hit in SyncTeX's own terms (all values in PDF points, top-left origin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ViewRecord {
    pub page: i32,
    /// Left edge of the enclosing box.
    pub h: f32,
    /// Baseline of the enclosing box.
    pub v: f32,
    pub width: f32,
    pub height: f32,
    pub depth: f32,
}

/// An inverse-search hit: (tag, line, column) of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EditRecord {
    pub tag: i32,
    pub line: i32,
    pub column: i32,
}

#[derive(Clone, Copy)]
struct Hit {
    h: i64,
    v: i64,
}

/// A node with a distance (`None` node means "nothing found").
#[derive(Clone, Copy)]
struct Nd {
    node: Option<u32>,
    distance: i64,
}

const ND_0: Nd = Nd {
    node: None,
    distance: INT_MAX,
};

impl Data {
    fn n(&self, id: u32) -> Option<&Node> {
        if id == NONE {
            None
        } else {
            self.nodes.get(id as usize)
        }
    }

    fn kind_of(&self, id: Option<u32>) -> Option<Kind> {
        id.and_then(|i| self.n(i)).map(|n| n.kind)
    }

    fn parent(&self, id: u32) -> u32 {
        self.n(id).map_or(NONE, |n| n.parent)
    }

    fn first_child(&self, id: u32) -> u32 {
        self.n(id).map_or(NONE, |n| n.first_child)
    }

    fn children(&self, id: u32) -> Children<'_> {
        Children {
            data: self,
            cur: self.first_child(id),
        }
    }

    fn has_child(&self, id: u32) -> bool {
        self.first_child(id) != NONE
    }

    // Field accessors with the reference implementation's per-kind data models: a field a
    // kind does not have reads as 0.

    fn data_h(&self, id: u32) -> i64 {
        self.n(id)
            .filter(|n| !matches!(n.kind, Kind::Sheet | Kind::Form))
            .map_or(0, |n| i64::from(n.h))
    }

    fn data_v(&self, id: u32) -> i64 {
        self.n(id)
            .filter(|n| !matches!(n.kind, Kind::Sheet | Kind::Form))
            .map_or(0, |n| i64::from(n.v))
    }

    fn data_width(&self, id: u32) -> i64 {
        self.n(id)
            .filter(|n| n.kind.is_box() || matches!(n.kind, Kind::Rule | Kind::Kern))
            .map_or(0, |n| i64::from(n.width))
    }

    fn data_height(&self, id: u32) -> i64 {
        self.n(id)
            .filter(|n| n.kind.is_box() || n.kind == Kind::Rule)
            .map_or(0, |n| i64::from(n.height))
    }

    fn data_depth(&self, id: u32) -> i64 {
        self.n(id)
            .filter(|n| n.kind.is_box() || n.kind == Kind::Rule)
            .map_or(0, |n| i64::from(n.depth))
    }

    /// Visible ("V") geometry: hboxes have it, other nodes use their plain fields.
    fn h_vis(&self, id: u32) -> i64 {
        match self.n(id) {
            Some(n) if n.kind == Kind::HBox => i64::from(n.h_v),
            _ => self.data_h(id),
        }
    }

    fn v_vis(&self, id: u32) -> i64 {
        match self.n(id) {
            Some(n) if n.kind == Kind::HBox => i64::from(n.v_v),
            _ => self.data_v(id),
        }
    }

    fn width_vis(&self, id: u32) -> i64 {
        match self.n(id) {
            Some(n) if n.kind == Kind::HBox => i64::from(n.width_v),
            _ => self.data_width(id),
        }
    }

    fn height_vis(&self, id: u32) -> i64 {
        match self.n(id) {
            Some(n) if n.kind == Kind::HBox => i64::from(n.height_v),
            _ => self.data_height(id),
        }
    }

    fn depth_vis(&self, id: u32) -> i64 {
        match self.n(id) {
            Some(n) if n.kind == Kind::HBox => i64::from(n.depth_v),
            _ => self.data_depth(id),
        }
    }

    /// `synctex_node_hbox_*`: visible geometry of hboxes, 0 for anything else.
    fn hbox_h(&self, id: u32) -> i64 {
        self.hbox_field(id, |n| n.h_v)
    }
    fn hbox_v(&self, id: u32) -> i64 {
        self.hbox_field(id, |n| n.v_v)
    }
    fn hbox_width(&self, id: u32) -> i64 {
        self.hbox_field(id, |n| n.width_v)
    }
    fn hbox_height(&self, id: u32) -> i64 {
        self.hbox_field(id, |n| n.height_v)
    }
    fn hbox_depth(&self, id: u32) -> i64 {
        self.hbox_field(id, |n| n.depth_v)
    }

    fn hbox_field(&self, id: u32, f: impl Fn(&Node) -> i32) -> i64 {
        match self.n(id) {
            Some(n) if n.kind == Kind::HBox => i64::from(f(n)),
            _ => 0,
        }
    }

    /// `synctex_node_tag` / `synctex_node_line`: form refs and containers have none.
    fn node_tag(&self, id: u32) -> i32 {
        self.n(id)
            .filter(|n| !matches!(n.kind, Kind::Sheet | Kind::Form | Kind::Ref))
            .map_or(0, |n| n.tag)
    }

    fn node_line(&self, id: u32) -> i32 {
        self.n(id)
            .filter(|n| !matches!(n.kind, Kind::Sheet | Kind::Form | Kind::Ref))
            .map_or(0, |n| n.line)
    }

    fn mean_line(&self, id: u32) -> i32 {
        let Some(n) = self.n(id) else { return 0 };
        if n.kind == Kind::HBox {
            return n.mean_line;
        }
        match self.n(n.parent) {
            Some(p) if p.kind == Kind::HBox => p.mean_line,
            _ => self.node_line(id),
        }
    }

    // -----------------------------------------------------------------------------------
    // Forward search.

    /// `synctex view` for one input tag: the records in the order the tool prints them.
    pub(crate) fn view(&self, tag: i32, line: i32, page_hint: i32) -> Vec<ViewRecord> {
        let max_line = self.max_line.get(&tag).copied().unwrap_or(0);
        let mut line = line.min(max_line);
        let mut offset: i32 = 1;
        for _ in 0..100 {
            if line > max_line {
                // The reference tool makes no more progress from here on.
                break;
            }
            if let Some(nodes) = self.display_query(tag, line, page_hint) {
                return nodes.into_iter().map(|id| self.view_record(id)).collect();
            }
            line = line.saturating_add(offset);
            offset = next_offset(offset);
            if line <= 0 {
                line = line.saturating_add(offset);
                offset = next_offset(offset);
            }
        }
        Vec::new()
    }

    fn display_query(&self, tag: i32, line: i32, page_hint: i32) -> Option<Vec<u32>> {
        if i64::from(tag) + i64::from(line) < 0 {
            return None;
        }
        let candidates = self.friends.get(&(tag, line))?;
        let page = |id: u32| self.n(id).map_or(-1, |n| n.page);
        let mut matches: Vec<u32> = candidates
            .iter()
            .copied()
            .filter(|&id| !self.kind_of(Some(id)).is_some_and(Kind::is_box))
            .take(MAX_MATCHES)
            .collect();
        if matches.is_empty() {
            matches = candidates.iter().copied().take(MAX_MATCHES).collect();
        }
        let (&first, rest) = matches.split_first()?;
        // Group by page: the first page's results in reverse order, then each further page.
        let mut groups: Vec<Vec<u32>> = vec![vec![first]];
        let page0 = page(first);
        let mut it = rest.iter().copied();
        for id in it.by_ref() {
            if page(id) == page0 {
                if let Some(g) = groups.first_mut() {
                    g.insert(0, id);
                }
            } else {
                groups.push(vec![id]);
                break;
            }
        }
        for id in it {
            let p = page(id);
            match groups
                .iter_mut()
                .find(|g| g.first().is_some_and(|&h| page(h) == p))
            {
                Some(g) => g.insert(1.min(g.len()), id),
                None => groups.insert(0, vec![id]),
            }
        }
        // Order the pages by distance to the hint, sort each page's results.
        let mut groups = groups.into_iter();
        let first_group = groups.next()?;
        let distance = |g: &Vec<u32>| {
            let p = g.first().map_or(-1, |&h| page(h));
            (i64::from(page_hint) - i64::from(p)).abs()
        };
        let mut best = distance(&first_group);
        let mut ordered = vec![self.vertically_sorted(&first_group, tag, line)];
        for g in groups {
            let d = distance(&g);
            let sorted = self.vertically_sorted(&g, tag, line);
            if d < best {
                ordered.insert(0, sorted);
                best = d;
            } else {
                ordered.insert(1.min(ordered.len()), sorted);
            }
        }
        Some(ordered.concat())
    }

    /// Keeps one result per enclosing hbox, weighted by how many children of that hbox
    /// belong to the searched line, in the reference tool's (not quite sorted) order.
    fn vertically_sorted(&self, group: &[u32], tag: i32, line: i32) -> Vec<u32> {
        // Number of children of a parent on the searched line (memoized: results under a
        // vbox all get the full count, results under an hbox only the first one does).
        let mut counts: HashMap<u32, i64> = HashMap::new();
        let mut weighted_hboxes: HashSet<u32> = HashSet::new();
        let mut weights = Vec::with_capacity(group.len());
        for &id in group {
            let parent = self.parent(id);
            let is_hbox = self.kind_of(Some(parent)) == Some(Kind::HBox);
            let w = if is_hbox && !weighted_hboxes.insert(parent) {
                0
            } else {
                *counts.entry(parent).or_insert_with(|| {
                    self.children(parent)
                        .filter(|&c| self.node_tag(c) == tag && self.node_line(c) == line)
                        .count() as i64
                })
            };
            weights.push(w);
        }
        let Some((&head, _)) = group.split_first() else {
            return Vec::new();
        };
        let mut list: Vec<(u32, i64)> = vec![(head, weights.first().copied().unwrap_or(0))];
        for (&id, &w) in group.iter().zip(weights.iter()).skip(1) {
            let head_w = list.first().map_or(0, |e| e.1);
            if w > head_w {
                list.insert(0, (id, w));
            } else if w == 0 {
                // Another result in an hbox that already has one.
            } else {
                // Insert after the first node (past the head) whose weight is <= w.
                let mut pos = 0;
                while pos + 1 < list.len() {
                    pos += 1;
                    if list.get(pos).is_some_and(|e| w < e.1) {
                        continue;
                    }
                    break;
                }
                list.insert((pos + 1).min(list.len()), (id, w));
            }
        }
        let cut = list
            .iter()
            .skip(1)
            .position(|e| e.1 == 0)
            .map_or(list.len(), |p| p + 1);
        list.truncate(cut);
        list.into_iter().map(|e| e.0).collect()
    }

    /// The box `synctex view` reports for a result node.
    fn box_visible(&self, id: u32) -> u32 {
        let mut node = id;
        if !self.kind_of(Some(node)).is_some_and(Kind::is_box) {
            let parent = self.parent(node);
            if parent == NONE {
                return NONE;
            }
            node = parent;
        }
        let bound = self.box_bound();
        let mean = i64::from(self.mean_line(node));
        let mut p = node;
        loop {
            p = self.parent(p);
            let Some(pn) = self.n(p) else { break };
            if pn.kind == Kind::HBox {
                if (mean - i64::from(self.mean_line(p))).abs() > 1 {
                    return node;
                } else if i64::from(pn.width) > bound
                    || i64::from(pn.height) + i64::from(pn.depth) > bound
                {
                    return p;
                }
                node = p;
            }
        }
        node
    }

    fn box_bound(&self) -> i64 {
        let mag = if self.magnification > 0 {
            self.magnification
        } else {
            1000
        };
        (1_500_000.0 / (f64::from(mag) / 1000.0)) as i64
    }

    fn view_record(&self, id: u32) -> ViewRecord {
        let b = self.box_visible(id);
        let unit = f64::from(self.unit);
        let size = |s: i64| (s as f64 * unit) as f32;
        ViewRecord {
            page: self.n(id).map_or(0, |n| n.page),
            h: (self.h_vis(b) as f64 * unit + f64::from(self.x_offset)) as f32,
            v: (self.v_vis(b) as f64 * unit + f64::from(self.y_offset)) as f32,
            width: size(self.width_vis(b)),
            height: size(self.height_vis(b)),
            depth: size(self.depth_vis(b)),
        }
    }

    // -----------------------------------------------------------------------------------
    // Inverse search.

    /// `synctex edit`: up to two candidate positions, best first.
    pub(crate) fn edit(&self, page: i32, x: f32, y: f32) -> Vec<EditRecord> {
        if self.unit <= 0.0 || !self.unit.is_finite() || !x.is_finite() || !y.is_finite() {
            return Vec::new();
        }
        let sheet = self
            .sheets
            .iter()
            .find(|s| s.page == page)
            .or_else(|| if page == 0 { self.sheets.first() } else { None });
        let Some(sheet) = sheet else {
            return Vec::new();
        };
        let unit = f64::from(self.unit);
        let hit = Hit {
            h: ((f64::from(x) - f64::from(self.x_offset)) / unit) as i64,
            v: ((f64::from(y) - f64::from(self.y_offset)) / unit) as i64,
        };
        let hit = Hit {
            h: hit.h.clamp(-INT_MAX, INT_MAX),
            v: hit.v.clamp(-INT_MAX, INT_MAX),
        };
        let mut hboxes = sheet.hboxes.iter().rev().copied();
        while let Some(hbox) = hboxes.next() {
            if !self.point_in_box(hit, hbox) {
                continue;
            }
            let mut node = hbox;
            for next in hboxes.by_ref() {
                if self.point_in_box(hit, next) {
                    node = self.smallest_container(next, node);
                }
            }
            let Some(node) = self.deepest_container_v2(hit, node) else {
                return Vec::new();
            };
            let (l, r) = self.closest_children_in_box(hit, node);
            return self.edit_result(l, r, node);
        }
        let first = self.first_child(sheet.node);
        if first == NONE {
            return Vec::new();
        }
        let l = self.closest_deep_child(hit, first);
        self.edit_result(
            l,
            Nd {
                node: None,
                distance: 0,
            },
            first,
        )
    }

    fn edit_result(&self, mut l: Nd, mut r: Nd, container: u32) -> Vec<EditRecord> {
        let rec = |id: u32| {
            self.n(id).map(|n| EditRecord {
                tag: n.tag,
                line: n.line,
                column: n.column,
            })
        };
        if let (Some(ln), Some(rn)) = (l.node, r.node) {
            let (lr, rr) = (rec(ln), rec(rn));
            if lr != rr {
                let (l_line, r_line) = (lr.map_or(0, |e| e.line), rr.map_or(0, |e| e.line));
                if r_line < l_line || (r_line == l_line && l.distance > r.distance) {
                    std::mem::swap(&mut l, &mut r);
                }
                return [l.node, r.node]
                    .into_iter()
                    .flatten()
                    .filter_map(rec)
                    .collect();
            }
            if l.distance > r.distance {
                l = r;
            }
        } else if r.node.is_some() {
            l = r;
        } else if l.node.is_none() {
            l.node = Some(container);
        }
        l.node.and_then(rec).into_iter().collect()
    }

    /// Signed horizontal distance from the hit to the node (positive: node to the right).
    fn h_ordered_distance(&self, hit: Hit, id: u32) -> i64 {
        let Some(n) = self.n(id) else { return INT_MAX };
        let span = |min: i64, width: i64| {
            let max = min + width.abs();
            if hit.h < min {
                min - hit.h
            } else if hit.h > max {
                max - hit.h
            } else {
                0
            }
        };
        match n.kind {
            Kind::VBox | Kind::VoidVBox | Kind::VoidHBox => {
                span(self.data_h(id), self.data_width(id))
            }
            Kind::HBox => span(self.hbox_h(id), self.hbox_width(id)),
            Kind::Kern => {
                let width = self.data_width(id);
                let h = self.data_h(id);
                let (min, max) = if width < 0 {
                    (h, h - width)
                } else {
                    (h - width, h)
                };
                let med = (min + max) / 2;
                if hit.h < min {
                    min - hit.h + 1
                } else if hit.h > max {
                    max - hit.h - 1
                } else if hit.h > med {
                    max - hit.h + 1
                } else {
                    min - hit.h - 1
                }
            }
            Kind::Rule | Kind::Glue | Kind::Math | Kind::Boundary | Kind::BoxBdry => {
                self.data_h(id) - hit.h
            }
            _ => INT_MAX,
        }
    }

    /// Signed vertical distance from the hit to the node (positive: node above the hit).
    fn v_ordered_distance(&self, hit: Hit, id: u32) -> i64 {
        let Some(n) = self.n(id) else { return INT_MAX };
        let span = |min: i64, height: i64, depth: i64| {
            let max = min + depth.abs();
            let min = min - height.abs();
            if hit.v < min {
                min - hit.v
            } else if hit.v > max {
                max - hit.v
            } else {
                0
            }
        };
        match n.kind {
            Kind::VBox | Kind::VoidVBox | Kind::VoidHBox => {
                span(self.data_v(id), self.data_height(id), self.data_depth(id))
            }
            Kind::HBox => span(self.hbox_v(id), self.hbox_height(id), self.hbox_depth(id)),
            Kind::Rule | Kind::Kern | Kind::Glue | Kind::Math => {
                let parent = n.parent;
                span(
                    self.data_v(id),
                    self.data_height(parent),
                    self.data_depth(parent),
                )
            }
            _ => INT_MAX,
        }
    }

    fn point_in_box(&self, hit: Hit, id: u32) -> bool {
        self.n(id).is_some()
            && self.h_ordered_distance(hit, id) == 0
            && self.v_ordered_distance(hit, id) == 0
    }

    /// The smaller of two overlapping hboxes (by area, then width, then height).
    fn smallest_container(&self, node: u32, other: u32) -> u32 {
        let w = self.hbox_width(node).abs();
        let ow = self.hbox_width(other).abs();
        let th = self.hbox_depth(node).abs() + self.hbox_height(node).abs();
        let oth = self.hbox_depth(other).abs() + self.hbox_height(other).abs();
        let area = i128::from(th) * i128::from(w);
        let other_area = i128::from(oth) * i128::from(ow);
        if area < other_area {
            return node;
        }
        if area > other_area {
            return other;
        }
        let (dw, odw) = (self.data_width(node).abs(), self.data_width(other).abs());
        if dw > odw {
            return node;
        }
        if dw < odw {
            return other;
        }
        if th < oth {
            return node;
        }
        if th > oth {
            return other;
        }
        node
    }

    /// Distance between a point and a box, taking the box's position into account.
    fn distance_to_box(hit: Hit, min_h: i64, max_h: i64, min_v: i64, max_v: i64) -> i64 {
        let dh = if hit.h < min_h {
            min_h - hit.h
        } else if hit.h <= max_h {
            0
        } else {
            hit.h - max_h
        };
        let dv = if hit.v < min_v {
            min_v - hit.v
        } else if hit.v <= max_v {
            0
        } else {
            hit.v - max_v
        };
        dh + dv
    }

    fn point_node_distance(&self, hit: Hit, id: u32) -> i64 {
        let Some(n) = self.n(id) else { return INT_MAX };
        match n.kind {
            Kind::VBox => {
                let min_h = self.data_h(id);
                let max_h = min_h + self.data_width(id).abs();
                let v = self.data_v(id);
                let max_v = v + self.data_depth(id).abs();
                let min_v = v - self.data_height(id).abs();
                Self::distance_to_box(hit, min_h, max_h, min_v, max_v)
            }
            Kind::HBox => {
                let min_h = self.hbox_h(id);
                let max_h = min_h + self.hbox_width(id).abs();
                let v = self.hbox_v(id);
                let max_v = v + self.hbox_depth(id).abs();
                let min_v = v - self.hbox_height(id).abs();
                Self::distance_to_box(hit, min_h, max_h, min_v, max_v)
            }
            Kind::VoidVBox | Kind::VoidHBox => {
                let h = self.data_h(id);
                let v = self.data_v(id);
                let max_v = v + self.data_depth(id).abs();
                let min_v = v - self.data_height(id).abs();
                let d = Self::distance_to_box(hit, h, h, min_v, max_v);
                let right = h + self.data_width(id).abs();
                let dd = Self::distance_to_box(hit, right, right, min_v, max_v);
                d.min(dd)
            }
            Kind::Kern => {
                let h = self.data_h(id);
                let max_v = self.data_v(id);
                let min_v = max_v - self.data_height(n.parent).abs();
                let d = Self::distance_to_box(hit, h, h, min_v, max_v);
                let other = h - self.data_width(id);
                let dd = Self::distance_to_box(hit, other, other, min_v, max_v);
                d.min(dd)
            }
            Kind::Glue | Kind::Math | Kind::Boundary | Kind::BoxBdry => {
                let h = self.data_h(id);
                let max_v = self.data_v(id);
                let min_v = max_v - self.data_height(n.parent).abs();
                Self::distance_to_box(hit, h, h, min_v, max_v)
            }
            _ => INT_MAX,
        }
    }

    /// The deepest box containing the hit, starting at `id` (which must have children).
    fn deepest_container_v2(&self, hit: Hit, id: u32) -> Option<u32> {
        if !self.has_child(id) {
            return None;
        }
        for child in self.children(id) {
            if self.point_in_box(hit, child)
                && let Some(deep) = self.deepest_container_v2(hit, child)
            {
                return Some(deep);
            }
        }
        if self.kind_of(Some(id)) == Some(Kind::VBox) {
            let mut best = ND_0;
            for child in self.children(id) {
                if self.has_child(child) {
                    let d = self.point_node_distance(hit, child);
                    if d <= best.distance {
                        best = Nd {
                            node: Some(child),
                            distance: d,
                        };
                    }
                }
            }
            if best.node.is_some() {
                return best.node;
            }
        }
        self.point_in_box(hit, id).then_some(id)
    }

    fn deepest_container_v3(&self, hit: Hit, id: u32) -> Nd {
        if !self.has_child(id) {
            return ND_0;
        }
        for child in self.children(id) {
            let deep = self.deepest_container_v3(hit, child);
            if deep.node.is_some() {
                return deep;
            }
        }
        if self.kind_of(Some(id)) == Some(Kind::VBox) {
            let mut best = ND_0;
            for child in self.children(id) {
                if self.has_child(child) {
                    let d = self.point_node_distance(hit, child);
                    if d < best.distance {
                        best = Nd {
                            node: Some(child),
                            distance: d,
                        };
                    }
                }
            }
            if best.node.is_some() {
                return best;
            }
        }
        if self.point_in_box(hit, id) {
            return Nd {
                node: Some(id),
                distance: 0,
            };
        }
        ND_0
    }

    /// The closest leaf below `id` (boxes are searched recursively, never returned).
    fn closest_deep_child(&self, hit: Hit, id: u32) -> Nd {
        let mut best = ND_0;
        for child in self.children(id) {
            let nd = if self.kind_of(Some(child)).is_some_and(Kind::is_box) {
                self.closest_deep_child(hit, child)
            } else {
                Nd {
                    node: Some(child),
                    distance: self.point_node_distance(hit, child),
                }
            };
            if nd.distance < best.distance
                || (nd.distance == best.distance && self.kind_of(nd.node) != Some(Kind::Kern))
            {
                best = nd;
            }
        }
        best
    }

    fn closest_children_in_box(&self, hit: Hit, id: u32) -> (Nd, Nd) {
        if !self.has_child(id) || self.kind_of(Some(id)) != Some(Kind::HBox) {
            // The reference implementation's vbox variant never finds anything.
            return (ND_0, ND_0);
        }
        let mut l = ND_0;
        let mut r = ND_0;
        let better = |cur: &Nd, child: u32| {
            let (Some(c), Some(n)) = (cur.node.and_then(|i| self.n(i)), self.n(child)) else {
                return false;
            };
            c.tag == n.tag && (c.line > n.line || (c.line == n.line && c.column > n.column))
        };
        for child in self.children(id) {
            let d = self.h_ordered_distance(hit, child);
            if d > 0 {
                // Closest node to the right; on ties prefer the earlier source position.
                if r.distance > d || (r.distance == d && better(&r, child)) {
                    r = Nd {
                        node: Some(child),
                        distance: d,
                    };
                }
            } else if d == 0 {
                if self.has_child(child) {
                    return self.closest_children_in_box(hit, child);
                }
                l = Nd {
                    node: Some(child),
                    distance: 0,
                };
            } else {
                let d = -d;
                if l.distance > d || (l.distance == d && better(&l, child)) {
                    l = Nd {
                        node: Some(child),
                        distance: d,
                    };
                }
            }
        }
        for side in [&mut l, &mut r] {
            if let Some(node) = side.node {
                let nd = self.deepest_container_v3(hit, node);
                if nd.node.is_some() {
                    *side = nd;
                }
                if let Some(node) = side.node {
                    let nd = self.closest_deep_child(hit, node);
                    if nd.node.is_some() {
                        side.node = nd.node;
                    }
                }
            }
        }
        (l, r)
    }
}

fn next_offset(offset: i32) -> i32 {
    if offset < 0 {
        offset.saturating_sub(1).saturating_neg()
    } else {
        offset.saturating_add(1).saturating_neg()
    }
}

struct Children<'a> {
    data: &'a Data,
    cur: u32,
}

impl Iterator for Children<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        let id = self.cur;
        let n = self.data.n(id)?;
        self.cur = n.next;
        Some(id)
    }
}
