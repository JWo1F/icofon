//! The two questions about a filled region that winding alone cannot answer.
//!
//! Most of what an icon asks for is a matter of turning contours the right way
//! round, which [`crate::svg`] does directly and cheaply. Two things are not:
//!
//! * **Even-odd over a contour that crosses itself.** Which side of an edge is
//!   ink depends on how many times the curve has wrapped around, and a
//!   self-crossing has both answers at once. The five-pointed star drawn as one
//!   stroke of the pen is the standard case: even-odd leaves the middle empty,
//!   and no re-winding of that single contour can say so.
//! * **`clip-path`.** A clip is an intersection, and the boundary of an
//!   intersection runs partly along one shape and partly along the other. It
//!   has to be cut where they cross.
//!
//! Both need the outlines resolved into a planar graph and walked again, which
//! is what `flo_curves` does. It is reached for only in those two cases: the
//! answer is exact but it re-cuts every curve it touches, so an icon that never
//! asks either question comes through untouched.

use flo_curves::bezier::path::{
  BezierPath, BezierPathFactory, SimpleBezierPath, path_intersect, path_remove_overlapped_points,
  path_sub,
};
use flo_curves::{Coord2, Coordinate2D};
use kurbo::{BezPath, ParamCurve, PathEl, Point, Rect, Shape};

/// How far the resolved outline may sit from the true one, in font units.
///
/// A twentieth of a unit out of the thousand an em is divided into, which is an
/// order finer than the quadratic fit that follows and far below what any
/// rasterizer resolves.
const ACCURACY: f64 = 0.05;

/// Fill `path` by the even-odd rule and give back its region wound for non-zero.
///
/// Returns `None` when the region comes out empty, which for a path that drew
/// something means the resolver could not make sense of it — the caller keeps
/// what it had rather than dropping the icon.
pub fn even_odd_region(path: &BezPath) -> Option<BezPath> {
  let input = to_curves(path);
  if input.is_empty() {
    return None;
  }
  let resolved: Vec<SimpleBezierPath> = path_remove_overlapped_points(&input, ACCURACY);
  to_bez(&resolved)
}

/// The part of `shape` that lies inside `clip`.
///
/// Returns `None` when nothing survives, which is a real answer — a shape
/// clipped entirely away draws nothing — so the caller drops the shape.
pub fn intersect(shape: &BezPath, clip: &BezPath) -> Option<BezPath> {
  let (shape, clip) = (to_curves(shape), to_curves(clip));
  if shape.is_empty() || clip.is_empty() {
    return None;
  }
  let clipped: Vec<SimpleBezierPath> = path_intersect(&shape, &clip, ACCURACY);
  to_bez(&clipped)
}

/// The part of `shape` that `cut` does not cover.
///
/// Returns `None` when nothing is left, which is a real answer: a shape cut
/// away entirely draws nothing.
pub fn subtract(shape: &BezPath, cuts: &[BezPath]) -> Option<BezPath> {
  let mut remainder = to_curves(shape);
  if remainder.is_empty() {
    return None;
  }
  // One cutter at a time: the resolver is documented as taking sides that do
  // not overlap themselves, and marks cut out of one body very well may.
  for cut in cuts {
    let cut = to_curves(cut);
    if cut.is_empty() {
      continue;
    }
    remainder = path_sub(&remainder, &cut, ACCURACY);
    if remainder.is_empty() {
      return None;
    }
  }
  to_bez(&remainder)
}

/// The rectangle `path` is, if it is one.
///
/// A clip that is a rectangle around everything it clips is doing nothing, and
/// that is the common case by a distance — every drawing tool writes one out to
/// clip its artboard. Recognising it keeps the resolver away from artwork that
/// had no question to ask.
pub fn as_rectangle(path: &BezPath) -> Option<Rect> {
  let mut corners: Vec<Point> = Vec::with_capacity(5);
  for element in path.elements() {
    match *element {
      PathEl::MoveTo(p) | PathEl::LineTo(p) => corners.push(p),
      PathEl::ClosePath => {}
      // A curve is never an axis-aligned rectangle's edge.
      _ => return None,
    }
  }
  // The closing edge may be written out or left implicit.
  if corners.len() == 5 && corners[4] == corners[0] {
    corners.pop();
  }
  if corners.len() != 4 {
    return None;
  }

  let bounds = path.bounding_box();
  // Every corner sits on a corner of the bounding box, and each edge runs along
  // one axis — which together leave only the rectangle itself.
  let on_corner =
    |p: &Point| (p.x == bounds.x0 || p.x == bounds.x1) && (p.y == bounds.y0 || p.y == bounds.y1);
  let axis_aligned = corners
    .iter()
    .zip(corners.iter().cycle().skip(1))
    .all(|(a, b)| a.x == b.x || a.y == b.y);
  if corners.iter().all(on_corner) && axis_aligned && bounds.area() > 0.0 {
    Some(bounds)
  } else {
    None
  }
}

/// Re-orient contours so that filling them by the non-zero rule fills what the
/// even-odd rule would.
///
/// TrueType only knows non-zero winding, so an even-odd path whose hole happens
/// to wind the same way as its outer contour would otherwise come out solid. A
/// contour nested an odd number of deep is a hole and must wind against its
/// container; one nested an even number of deep is solid and must wind with the
/// outermost contours.
///
/// This is also what gives a resolved region its winding: what comes back from
/// a boolean operation is a set of simple contours, correctly nested but with
/// no direction to say which of them are holes.
pub fn even_odd_by_nesting(path: &BezPath) -> BezPath {
  let contours = split_contours(path);
  if contours.len() < 2 {
    return path.clone();
  }

  let mut out = BezPath::new();
  for (index, contour) in contours.iter().enumerate() {
    let Some(probe) = first_point(contour) else {
      continue;
    };
    let depth = contours
      .iter()
      .enumerate()
      .filter(|(other_index, other)| *other_index != index && other.contains(probe))
      .count();

    // Contours at even depth all share one orientation, odd depth the other.
    if (depth % 2 == 0) == (contour.area() > 0.0) {
      out.extend(contour.iter());
    } else {
      out.extend(contour.reverse_subpaths());
    }
  }
  out
}

/// Split a path into one `BezPath` per contour.
fn split_contours(path: &BezPath) -> Vec<BezPath> {
  let mut contours = Vec::new();
  let mut current = BezPath::new();
  for element in path.elements() {
    if matches!(element, PathEl::MoveTo(_)) && !current.is_empty() {
      contours.push(std::mem::take(&mut current));
    }
    current.push(*element);
  }
  if !current.is_empty() {
    contours.push(current);
  }
  contours
}

fn first_point(contour: &BezPath) -> Option<Point> {
  match contour.elements().first() {
    Some(PathEl::MoveTo(p)) => Some(*p),
    _ => None,
  }
}

/// Split a path into one closed `flo_curves` path per contour.
///
/// Everything is raised to cubics, which is the only segment `flo_curves`
/// carries, and every contour is closed explicitly: a region is bounded by
/// definition, and an open contour would leave the resolver to guess.
fn to_curves(path: &BezPath) -> Vec<SimpleBezierPath> {
  let at = |p: Point| Coord2(p.x, p.y);
  // A line as a cubic, with its controls a third and two thirds along. Putting
  // them on the ends instead would leave the curve with no direction there, and
  // the resolver reads tangents to decide which side of an edge it is on.
  let straight = |from: Point, to: Point| {
    (
      Coord2(
        from.x + (to.x - from.x) / 3.0,
        from.y + (to.y - from.y) / 3.0,
      ),
      Coord2(
        from.x + 2.0 * (to.x - from.x) / 3.0,
        from.y + 2.0 * (to.y - from.y) / 3.0,
      ),
    )
  };
  // A quadratic is the cubic whose controls sit two thirds of the way out.
  let raised = |from: Point, control: Point, to: Point| {
    (
      Coord2(
        from.x + 2.0 / 3.0 * (control.x - from.x),
        from.y + 2.0 / 3.0 * (control.y - from.y),
      ),
      Coord2(
        to.x + 2.0 / 3.0 * (control.x - to.x),
        to.y + 2.0 / 3.0 * (control.y - to.y),
      ),
    )
  };

  let mut out = Vec::new();
  let mut start = Point::ZERO;
  let mut current = Point::ZERO;
  let mut edges: Vec<(Coord2, Coord2, Coord2)> = Vec::new();

  let finish = |start: Point, current: Point, edges: &mut Vec<_>, out: &mut Vec<_>| {
    if current != start {
      let (c1, c2) = straight(current, start);
      edges.push((c1, c2, at(start)));
    }
    if !edges.is_empty() {
      out.push(SimpleBezierPath::from_points(
        at(start),
        std::mem::take(edges),
      ));
    }
  };

  for element in path.elements() {
    match *element {
      PathEl::MoveTo(p) => {
        finish(start, current, &mut edges, &mut out);
        start = p;
        current = p;
      }
      PathEl::LineTo(p) => {
        let (c1, c2) = straight(current, p);
        edges.push((c1, c2, at(p)));
        current = p;
      }
      PathEl::QuadTo(c, p) => {
        let (c1, c2) = raised(current, c, p);
        edges.push((c1, c2, at(p)));
        current = p;
      }
      PathEl::CurveTo(c1, c2, p) => {
        // Halved, so that no segment runs from one extreme of a curve to
        // another. A circle written as four quarter-arcs has its corners
        // exactly at its extremes, which is exactly where an axis-aligned clip
        // meets it — and the resolver cannot tell a crossing from a corner.
        let whole = kurbo::CubicBez::new(current, c1, c2, p);
        let first = whole.subsegment(0.0..0.5);
        let second = whole.subsegment(0.5..1.0);
        edges.push((at(first.p1), at(first.p2), at(first.p3)));
        edges.push((at(second.p1), at(second.p2), at(second.p3)));
        current = p;
      }
      PathEl::ClosePath => {
        if current != start {
          let (c1, c2) = straight(current, start);
          edges.push((c1, c2, at(start)));
        }
        current = start;
      }
    }
  }
  finish(start, current, &mut edges, &mut out);
  out
}

/// Gather resolved contours back into one path, or `None` if there are none.
fn to_bez(paths: &[SimpleBezierPath]) -> Option<BezPath> {
  let mut out = BezPath::new();
  for path in paths {
    let start = path.start_point();
    out.move_to(Point::new(start.x(), start.y()));
    for (c1, c2, end) in path.points() {
      out.curve_to(
        Point::new(c1.x(), c1.y()),
        Point::new(c2.x(), c2.y()),
        Point::new(end.x(), end.y()),
      );
    }
    out.close_path();
  }
  (!out.is_empty()).then_some(even_odd_by_nesting(&out))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The pentagram from `examples/icons/geometry/polygon-star.svg`: one contour
  /// that crosses itself five times.
  fn star() -> BezPath {
    let mut path = BezPath::new();
    path.move_to((12.0, 2.0));
    for point in [(4.6, 21.0), (21.4, 8.9), (2.6, 8.9), (19.4, 21.0)] {
      path.line_to(point);
    }
    path.close_path();
    path
  }

  fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.01)
  }

  #[test]
  fn even_odd_empties_the_middle_of_a_star() {
    let region = even_odd_region(&star()).expect("a star is not empty");
    // Five points, and nothing in the middle: the centre is wrapped twice.
    assert_eq!(
      region
        .elements()
        .iter()
        .filter(|el| matches!(el, PathEl::MoveTo(_)))
        .count(),
      5
    );
    assert_eq!(
      region.winding(Point::new(12.0, 11.0)),
      0,
      "the middle is a hole"
    );
    assert_ne!(
      region.winding(Point::new(12.0, 4.0)),
      0,
      "the top point is ink"
    );
  }

  #[test]
  fn a_clip_cuts_the_shape_down_to_the_overlap() {
    let circle = kurbo::Circle::new((12.0, 12.0), 9.0).to_path(0.01);
    let clipped = intersect(&circle, &rect(0.0, 0.0, 24.0, 12.0)).expect("half a disc is left");
    let bounds = clipped.bounding_box();
    assert!(
      bounds.y0 > 2.9 && bounds.y1 < 12.1,
      "clipped to the top half: {bounds:?}"
    );
    assert_ne!(
      clipped.winding(Point::new(12.0, 8.0)),
      0,
      "the kept half is ink"
    );
    assert_eq!(
      clipped.winding(Point::new(12.0, 16.0)),
      0,
      "the cut half is gone"
    );
  }

  #[test]
  fn a_mark_cut_out_of_a_body_leaves_a_hole() {
    let body = rect(0.0, 0.0, 24.0, 24.0);
    let mark = kurbo::Circle::new((12.0, 12.0), 5.0).to_path(0.01);
    let cut = subtract(&body, std::slice::from_ref(&mark)).expect("most of the body survives");
    assert_eq!(cut.winding(Point::new(12.0, 12.0)), 0, "the mark is a hole");
    assert_ne!(cut.winding(Point::new(2.0, 2.0)), 0, "the corners are ink");
  }

  #[test]
  fn a_shape_cut_away_entirely_is_nothing() {
    let small = kurbo::Circle::new((12.0, 12.0), 2.0).to_path(0.01);
    assert!(subtract(&small, &[rect(0.0, 0.0, 24.0, 24.0)]).is_none());
  }

  #[test]
  fn a_shape_clipped_away_entirely_is_nothing() {
    let circle = kurbo::Circle::new((12.0, 12.0), 4.0).to_path(0.01);
    assert!(intersect(&circle, &rect(30.0, 30.0, 40.0, 40.0)).is_none());
  }

  #[test]
  fn a_rectangle_is_recognized_however_it_was_written() {
    let bounds = Rect::new(0.0, 0.0, 24.0, 12.0);
    assert_eq!(as_rectangle(&bounds.to_path(0.01)), Some(bounds));

    // Written the long way round, closing edge spelled out.
    let mut spelled = BezPath::new();
    spelled.move_to((0.0, 0.0));
    for point in [(24.0, 0.0), (24.0, 12.0), (0.0, 12.0), (0.0, 0.0)] {
      spelled.line_to(point);
    }
    spelled.close_path();
    assert_eq!(as_rectangle(&spelled), Some(bounds));

    // And things that only pass for one.
    assert_eq!(
      as_rectangle(&kurbo::Circle::new((0.0, 0.0), 5.0).to_path(0.01)),
      None
    );
    assert_eq!(as_rectangle(&star()), None);
    let mut diamond = BezPath::new();
    diamond.move_to((12.0, 0.0));
    for point in [(24.0, 12.0), (12.0, 24.0), (0.0, 12.0)] {
      diamond.line_to(point);
    }
    diamond.close_path();
    assert_eq!(
      as_rectangle(&diamond),
      None,
      "a diamond fills the same bounds"
    );
  }
}
