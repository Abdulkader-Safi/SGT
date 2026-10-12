//! Lane layout for the commit graph. Feed it commits in the order `git log`
//! delivers them (children before parents) and it returns one row each.

use crate::model::Oid;

/// Lane colours are indexes into this many theme colours.
pub const LANE_COLORS: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// A lane that passes this row from top to bottom.
    Through,
    /// From this row's commit down to the lane of one of its parents.
    ToParent,
    /// From a lane at the top of the row into this row's commit.
    FromChild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from_lane: u16,
    pub to_lane: u16,
    pub color: u8,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    /// Column of this commit's dot.
    pub lane: u16,
    pub color: u8,
    pub edges: Vec<Edge>,
}

/// A lane held for a commit that a child above is waiting for.
struct Lane {
    waiting_for: Oid,
    color: u8,
}

#[derive(Default)]
pub struct GraphLayout {
    /// `None` is a free lane.
    lanes: Vec<Option<Lane>>,
    next_color: u8,
}

impl GraphLayout {
    // ponytail: scans every active lane per commit. Lanes number in the tens
    // even for wide histories. Index them by commit ID if that ever changes.
    pub fn push(&mut self, oid: &Oid, parents: &[Oid]) -> GraphRow {
        let mut edges = Vec::new();

        // The commit takes the leftmost lane reserved for it. Other lanes
        // reserved for it merge in, and every other lane passes through.
        let mut mine = None;
        for (i, lane) in self.lanes.iter().enumerate() {
            let Some(lane) = lane else { continue };
            let kind = if lane.waiting_for == *oid {
                mine.get_or_insert((i, lane.color));
                EdgeKind::FromChild
            } else {
                EdgeKind::Through
            };
            let to = if kind == EdgeKind::FromChild {
                mine.unwrap().0
            } else {
                i
            };
            edges.push(Edge {
                from_lane: i as u16,
                to_lane: to as u16,
                color: lane.color,
                kind,
            });
        }
        let (lane, color) = mine.unwrap_or_else(|| (self.first_free(), self.new_color()));

        // Lanes that end in this row stay taken until the parents are placed,
        // so a parent's edge never leaves through a lane a child's edge just
        // came in on.
        for (n, parent) in parents.iter().enumerate() {
            let (to, to_color) = if n == 0 {
                self.reserve(lane, parent, color);
                (lane, color)
            } else if let Some(found) = self.reserved_for(parent) {
                found
            } else {
                let found = (self.first_free(), self.new_color());
                self.reserve(found.0, parent, found.1);
                found
            };
            edges.push(Edge {
                from_lane: lane as u16,
                to_lane: to as u16,
                color: to_color,
                kind: EdgeKind::ToParent,
            });
        }

        for slot in &mut self.lanes {
            if slot.as_ref().is_some_and(|l| l.waiting_for == *oid) {
                *slot = None;
            }
        }
        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }

        GraphRow {
            lane: lane as u16,
            color,
            edges,
        }
    }

    fn first_free(&self) -> usize {
        self.lanes
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.lanes.len())
    }

    fn reserved_for(&self, oid: &Oid) -> Option<(usize, u8)> {
        self.lanes.iter().enumerate().find_map(|(i, lane)| {
            let lane = lane.as_ref()?;
            (lane.waiting_for == *oid).then_some((i, lane.color))
        })
    }

    fn reserve(&mut self, lane: usize, oid: &Oid, color: u8) {
        if lane == self.lanes.len() {
            self.lanes.push(None);
        }
        self.lanes[lane] = Some(Lane {
            waiting_for: oid.clone(),
            color,
        });
    }

    fn new_color(&mut self) -> u8 {
        let color = self.next_color;
        self.next_color = (color + 1) % LANE_COLORS;
        color
    }
}

#[cfg(test)]
mod tests {
    use super::EdgeKind::{FromChild, Through, ToParent};
    use super::*;

    /// Lays out `(commit, parents)` pairs given in log order.
    fn layout(history: &[(&str, &[&str])]) -> Vec<GraphRow> {
        let mut graph = GraphLayout::default();
        history
            .iter()
            .map(|(oid, parents)| {
                let parents: Vec<Oid> = parents.iter().map(|p| Oid::from(*p)).collect();
                graph.push(&Oid::from(*oid), &parents)
            })
            .collect()
    }

    fn row(lane: u16, color: u8, edges: &[(u16, u16, u8, EdgeKind)]) -> GraphRow {
        let edges = edges
            .iter()
            .map(|&(from_lane, to_lane, color, kind)| Edge {
                from_lane,
                to_lane,
                color,
                kind,
            })
            .collect();
        GraphRow { lane, color, edges }
    }

    #[test]
    fn linear() {
        let rows = layout(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
        assert_eq!(
            rows,
            [
                row(0, 0, &[(0, 0, 0, ToParent)]),
                row(0, 0, &[(0, 0, 0, FromChild), (0, 0, 0, ToParent)]),
                row(0, 0, &[(0, 0, 0, FromChild)]),
            ]
        );
    }

    #[test]
    fn single_merge() {
        let rows = layout(&[
            ("m", &["a2", "b1"]),
            ("a2", &["a1"]),
            ("b1", &["a1"]),
            ("a1", &[]),
        ]);
        assert_eq!(
            rows,
            [
                row(0, 0, &[(0, 0, 0, ToParent), (0, 1, 1, ToParent)]),
                row(
                    0,
                    0,
                    &[
                        (0, 0, 0, FromChild),
                        (1, 1, 1, Through),
                        (0, 0, 0, ToParent)
                    ]
                ),
                row(
                    1,
                    1,
                    &[
                        (0, 0, 0, Through),
                        (1, 1, 1, FromChild),
                        (1, 1, 1, ToParent)
                    ]
                ),
                // Both lanes waited for a1. It takes the left one and the
                // right one merges in.
                row(0, 0, &[(0, 0, 0, FromChild), (1, 0, 1, FromChild)]),
            ]
        );
    }

    #[test]
    fn octopus() {
        let rows = layout(&[
            ("o", &["a", "b", "c"]),
            ("a", &["r"]),
            ("b", &["r"]),
            ("c", &["r"]),
            ("r", &[]),
        ]);
        assert_eq!(
            rows[0],
            row(
                0,
                0,
                &[
                    (0, 0, 0, ToParent),
                    (0, 1, 1, ToParent),
                    (0, 2, 2, ToParent)
                ]
            )
        );
        assert_eq!(
            rows.iter().map(|r| r.lane).collect::<Vec<_>>(),
            [0, 0, 1, 2, 0]
        );
        assert_eq!(
            rows[4],
            row(
                0,
                0,
                &[
                    (0, 0, 0, FromChild),
                    (1, 0, 1, FromChild),
                    (2, 0, 2, FromChild)
                ]
            )
        );
    }

    #[test]
    fn criss_cross() {
        let rows = layout(&[
            ("m1", &["a", "b"]),
            ("m2", &["b", "a"]),
            ("a", &["r"]),
            ("b", &["r"]),
            ("r", &[]),
        ]);
        assert_eq!(
            rows,
            [
                row(0, 0, &[(0, 0, 0, ToParent), (0, 1, 1, ToParent)]),
                // m2 has no child, so it opens lane 2. Its second parent
                // already has lane 0.
                row(
                    2,
                    2,
                    &[
                        (0, 0, 0, Through),
                        (1, 1, 1, Through),
                        (2, 2, 2, ToParent),
                        (2, 0, 0, ToParent)
                    ]
                ),
                row(
                    0,
                    0,
                    &[
                        (0, 0, 0, FromChild),
                        (1, 1, 1, Through),
                        (2, 2, 2, Through),
                        (0, 0, 0, ToParent)
                    ]
                ),
                row(
                    1,
                    1,
                    &[
                        (0, 0, 0, Through),
                        (1, 1, 1, FromChild),
                        (2, 1, 2, FromChild),
                        (1, 1, 1, ToParent)
                    ]
                ),
                row(0, 0, &[(0, 0, 0, FromChild), (1, 0, 1, FromChild)]),
            ]
        );
    }

    #[test]
    fn a_freed_lane_is_reused_and_colours_cycle() {
        // Ten unrelated one-commit histories all sit in lane 0.
        let history: Vec<(String, &[&str])> = (0..10).map(|i| (format!("t{i}"), &[][..])).collect();
        let history: Vec<(&str, &[&str])> = history.iter().map(|(o, p)| (o.as_str(), *p)).collect();
        let rows = layout(&history);
        assert!(rows.iter().all(|r| r.lane == 0 && r.edges.is_empty()));
        assert_eq!(
            rows.iter().map(|r| r.color).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5, 6, 7, 0, 1]
        );
    }
}
