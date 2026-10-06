//! Column A, top: clock, date and the month calendar.

use super::*;

pub(super) const CLOCK_BASELINE: f32 = 172.0;
pub(super) const DATE_Y: f32 = 206.0;

pub(super) fn draw_clock(cv: &mut Canvas, now: &DateTime<Local>, x: f32, right: f32) {
    let h12 = if now.hour() % 12 == 0 { 12 } else { now.hour() % 12 };
    let hm = format!("{}:{:02}", h12, now.minute());
    let ampm = if now.hour() < 12 { "AM" } else { "PM" };
    let sec_font = num(34.0, Face::NumLight);
    let mut f = num(170.0, Face::NumLight);
    for size in (120..=170).rev().step_by(5) {
        // 1-9 o'clock can be larger than 10-12
        f = num(size as f32, Face::NumLight);
        if x + cv.text_len(&hm, f) + 16.0 + cv.text_len("00", sec_font) <= right {
            break;
        }
    }
    cv.text_at(x - 4.0, CLOCK_BASELINE, &hm, f, INK, Anchor::Ls);
    let digits_top = CLOCK_BASELINE + cv.ink_top("0", f);
    let sx = x + cv.text_len(&hm, f) + 16.0;
    cv.caps(sx, digits_top + 4.0, ampm, 20.0, Some(ACCENT), 2.5, false);
    cv.text(sx, digits_top + 30.0, &format!("{:02}", now.second()), sec_font, mix(ACCENT, BG, 0.7));
    let date = format!("{}. {:02}. {:02}   {}요일", now.year(), now.month(), now.day(), WEEKDAYS[now.weekday().num_days_from_monday() as usize]);
    cv.text(x, DATE_Y, &date, ko(24.0, false), MUTED);
}

pub(super) fn draw_calendar(cv: &mut Canvas, now: &DateTime<Local>, x: f32, y: f32, w: f32) {
    let cw = w / 7.0;
    cv.caps(x + 4.0, y, &now.format("%B").to_string(), 16.0, Some(MUTED), 2.5, false);
    let (f_head, f_day) = (ko(15.0, false), num(21.0, Face::Num));
    for (i, name) in ["일", "월", "화", "수", "목", "금", "토"].iter().enumerate() {
        let c = if i == 0 { SUNDAY } else if i == 6 { SATURDAY } else { MUTED };
        cv.text_at(x + cw * i as f32 + cw / 2.0, y + 32.0, name, f_head, c, Anchor::Mt);
    }
    // Sunday-first month grid
    let first = now.with_day(1).unwrap();
    let lead = first.weekday().num_days_from_sunday() as usize;
    let days = {
        let (ny, nm) = if now.month() == 12 { (now.year() + 1, 1) } else { (now.year(), now.month() + 1) };
        chrono::NaiveDate::from_ymd_opt(ny, nm, 1).unwrap().pred_opt().unwrap().day() as usize
    };
    for day in 1..=days {
        let cell = lead + day - 1;
        let (r, i) = (cell / 7, cell % 7);
        let cy = y + 70.0 + r as f32 * 27.0;
        let cx = x + cw * i as f32 + cw / 2.0;
        let red = i == 0 || is_holiday(now.year(), now.month(), day as u32);
        let mut color = if red { SUNDAY } else if i == 6 { SATURDAY } else { INK };
        if day as u32 == now.day() {
            cv.rect(cx - 15.0, cy - 13.0, cx + 15.0, cy + 13.0, ACCENT);
            color = BG;
        }
        cv.text_at(cx, cy, &day.to_string(), f_day, color, Anchor::Mm);
    }
}
