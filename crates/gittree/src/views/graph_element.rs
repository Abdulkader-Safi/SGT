use gittree_core::graph::{EdgeKind, GraphRow};
use gpui_kit::{
    Bounds, Hsla, IntoElement, PathBuilder, Pixels, Styled, Window, canvas, fill, point, px, quad,
    size,
};

pub const ROW_HEIGHT: Pixels = px(28.);
pub const LANE_WIDTH: Pixels = px(16.);
const LINE_WIDTH: Pixels = px(2.);
const DOT_SIZE: Pixels = px(10.);

/// The graph column of one row: its lanes, edges and commit dot.
pub fn graph_cell(
    row: GraphRow,
    lanes: usize,
    colors: [Hsla; 8],
    background: Hsla,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| paint_row(&row, bounds, colors, background, window),
    )
    .w(LANE_WIDTH * lanes as f32)
    .h_full()
    .flex_none()
}

fn paint_row(
    row: &GraphRow,
    bounds: Bounds<Pixels>,
    colors: [Hsla; 8],
    background: Hsla,
    window: &mut Window,
) {
    let x = |lane: u16| bounds.left() + LANE_WIDTH * (lane as f32 + 0.5);
    let (top, mid, bottom) = (bounds.top(), bounds.center().y, bounds.bottom());

    for edge in &row.edges {
        let color = colors[edge.color as usize % 8];
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

    // A merge commit is a ring. Every other commit is a filled dot.
    let merge = row
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::ToParent)
        .count()
        > 1;
    let color = colors[row.color as usize % 8];
    let dot = Bounds::centered_at(point(x(row.lane), mid), size(DOT_SIZE, DOT_SIZE));
    window.paint_quad(quad(
        dot,
        DOT_SIZE / 2.,
        if merge { background } else { color },
        px(2.),
        color,
        Default::default(),
    ));
}
