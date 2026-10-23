//! The stroke icons of the Settings sidebar, drawn with the painter on a 16 by 16 grid.

use egui::epaint::CubicBezierShape;
use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2};

use crate::settings::Section;

pub(crate) fn paint(painter: &Painter, rect: Rect, section: Section, color: Color32) {
    let scale = rect.width() / 16.0;
    let at = |x: f32, y: f32| rect.min + pos2(x, y).to_vec2() * scale;
    let stroke = Stroke::new(1.5, color);
    let line = |points: &[(f32, f32)]| {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
        painter.add(egui::Shape::line(points, stroke));
    };
    let circle = |x: f32, y: f32, r: f32| {
        painter.circle_stroke(at(x, y), r * scale, stroke);
    };
    let curve = |points: [(f32, f32); 4]| {
        let [a, b, c, d] = points.map(|(x, y)| at(x, y));
        painter.add(CubicBezierShape::from_points_stroke(
            [a, b, c, d],
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
    };
    match section {
        Section::General => {
            circle(8.0, 8.0, 2.5);
            for (from, to) in [
                ((8.0, 1.5), (8.0, 3.5)),
                ((8.0, 12.5), (8.0, 14.5)),
                ((1.5, 8.0), (3.5, 8.0)),
                ((12.5, 8.0), (14.5, 8.0)),
                ((3.4, 3.4), (4.8, 4.8)),
                ((11.2, 11.2), (12.6, 12.6)),
                ((3.4, 12.6), (4.8, 11.2)),
                ((11.2, 4.8), (12.6, 3.4)),
            ] {
                line(&[from, to]);
            }
        }
        Section::Profiles => {
            circle(8.0, 5.5, 2.5);
            curve([(3.0, 14.0), (3.0, 10.5), (13.0, 10.5), (13.0, 14.0)]);
        }
        Section::Workflows => {
            for x in [3.0, 8.0, 13.0] {
                circle(x, 8.0, 1.8);
            }
        }
        Section::Trust => {
            line(&[(8.0, 1.5), (13.5, 4.0), (13.5, 8.0)]);
            curve([(13.5, 8.0), (13.5, 11.0), (11.0, 13.5), (8.0, 14.5)]);
            curve([(8.0, 14.5), (5.0, 13.5), (2.5, 11.0), (2.5, 8.0)]);
            line(&[(2.5, 8.0), (2.5, 4.0), (8.0, 1.5)]);
        }
        Section::Shortcuts => {
            painter.rect_stroke(
                Rect::from_min_size(at(1.5, 4.0), egui::vec2(13.0, 8.0) * scale),
                1.5 * scale,
                stroke,
                egui::StrokeKind::Middle,
            );
            for keys in
                [[(4.0, 7.0), (5.0, 7.0)], [(7.0, 7.0), (9.0, 7.0)], [(11.0, 7.0), (12.0, 7.0)]]
            {
                line(&keys);
            }
            line(&[(5.0, 9.5), (11.0, 9.5)]);
        }
        Section::WorktreeSetup => {
            painter.rect_stroke(
                Rect::from_min_size(at(1.5, 2.5), egui::vec2(8.0, 8.0) * scale),
                1.5 * scale,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.rect_stroke(
                Rect::from_min_size(at(6.5, 5.5), egui::vec2(8.0, 8.0) * scale),
                1.5 * scale,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        Section::HouseStyle => {
            curve([(2.0, 8.0), (3.5, 6.0), (5.0, 6.0), (6.5, 8.0)]);
            curve([(6.5, 8.0), (8.0, 10.0), (9.5, 10.0), (11.0, 8.0)]);
            curve([(11.0, 8.0), (12.5, 6.0), (13.5, 6.0), (14.0, 7.0)]);
        }
    }
}
