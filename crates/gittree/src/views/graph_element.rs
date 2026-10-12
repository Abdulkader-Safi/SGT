use gittree_core::graph::{EdgeKind, GraphRow};
use gpui_kit::{
    Bounds, Hsla, IntoElement, PathBuilder, Pixels, Styled, Window, canvas, fill, point, px, quad,
    rgb, size,
};

pub const ROW_HEIGHT: Pixels = px(24.);
pub const LANE_WIDTH: Pixels = px(14.);
const LINE_WIDTH: Pixels = px(2.);
const DOT_SIZE: Pixels = px(9.);

// Moves to the theme in M1.
const LANE_COLORS: [u32; 8] = [
    0x3b82f6, 0xf59e0b, 0x10b981, 0xef4444, 0x8b5cf6, 0x06b6d4, 0xec4899, 0x84cc16,
];

/// The graph column of one row: its lanes, edges and commit dot.
pub fn graph_cell(row: GraphRow, lanes: usize, background: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| paint_row(&row, bounds, background, window),
    )
    .w(LANE_WIDTH * lanes as f32)
    .h_full()
    .flex_none()
}

fn paint_row(row: &GraphRow, bounds: Bounds<Pixels>, background: Hsla, window: &mut Window) {
    let x = |lane: u16| bounds.left() + LANE_WIDTH * (lane as f32 + 0.5);
    let (top, mid, bottom) = (bounds.top(), bounds.center().y, bounds.bottom());

    for edge in &row.edges {
        let color = rgb(LANE_COLORS[edge.color as usize]);
        let (from, to) = (x(edge.from_lane), x(edge.to_lane));
        let (start, end) = match edge.kind {
            EdgeKind::Through => (point(from, top), point(to, bottom)),
            EdgeKind::FromChild => (point(from, top), point(to, mid)),
            EdgeKind::ToParent => (point(from, mid), point(to, bottom)),
        };
        if from == to {
            // Most edges are vertical, and a quad is cheaper than a path.
            let half = LINE_WIDTH / 2.;
            window.paint_quad(fill(
                Bounds::from_corners(point(from - half, start.y), point(from + half, end.y)),
                color,
            ));
            continue;
        }
        // The control point is in the lane that carries on past this row,
        // level with the dot, so the curve joins that lane going straight
        // up or down.
        let control = match edge.kind {
            EdgeKind::FromChild => point(from, mid),
            EdgeKind::ToParent | EdgeKind::Through => point(to, start.y),
        };
        let mut path = PathBuilder::stroke(LINE_WIDTH);
        path.move_to(start);
        path.curve_to(end, control);
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }

    let dot = Bounds::centered_at(point(x(row.lane), mid), size(DOT_SIZE, DOT_SIZE));
    window.paint_quad(quad(
        dot,
        DOT_SIZE / 2.,
        rgb(LANE_COLORS[row.color as usize]),
        px(1.5),
        background,
        Default::default(),
    ));
}
