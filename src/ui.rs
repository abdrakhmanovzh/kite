//! Palette, formatting and text helpers shared by the views.

use std::ops::Range;

use chrono::{DateTime, Datelike, Local, TimeZone};
use gpui_kit::{
    FontStyle, FontWeight, HighlightStyle, Hsla, StrikethroughStyle, UnderlineStyle, px, rgb, rgba,
};
use tdlib_rs::{
    enums::{TextEntityType, UserStatus},
    types::FormattedText,
};

// ---------------------------------------------------------------------------
// Design tokens. Every view draws from these; avoid raw values in views.
// ---------------------------------------------------------------------------

// Every color token carries its dark and light value; `hsla` picks one for
// the current appearance, so views never branch on the theme.
//
// Light mode is its own design, not an inversion: a soft neutral backdrop
// with white panels, Telegram's vivid blue for accents and the selected
// chat, and tinted outgoing bubbles. Sender names get darker variants so
// they read on light bubbles.

/// A design token: one color per appearance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub dark: u32,
    pub light: u32,
}

const fn color(dark: u32, light: u32) -> Color {
    Color { dark, light }
}

/// Anything `hsla` can paint: a token, or a raw 0xRRGGBB used in both themes.
pub trait Paint: Copy {
    fn rgb(self) -> u32;
}

impl Paint for Color {
    fn rgb(self) -> u32 {
        if is_light() { self.light } else { self.dark }
    }
}

impl Paint for u32 {
    fn rgb(self) -> u32 {
        self
    }
}

// Surfaces, back to front.
/// Window backdrop; the sidebar sits directly on it.
pub const WINDOW: Color = color(0x16191d, 0xf5f6f8);
/// Panels: conversation, side panels.
pub const CANVAS: Color = color(0x1a1e24, 0xffffff);
/// Cards and footers on a panel.
pub const BASE: Color = color(0x1e2227, 0xf7f8fa);
/// Floating layers: menus, palette, dialogs, popovers.
pub const OVERLAY: Color = color(0x252a30, 0xffffff);
/// Controls on a surface: fields, chips, key caps.
pub const RAISED: Color = color(0x2c3137, 0xeff1f4);
/// Hover on the backdrop and panels.
pub const HOVER: Color = color(0x262b31, 0xe9ecf0);
/// Hover on OVERLAY or RAISED.
pub const HOVER_RAISED: Color = color(0x31373e, 0xe4e7ec);
/// Hairlines between regions.
pub const STROKE: Color = color(0x2a2f35, 0xe3e6eb);
/// Borders of floating layers and controls.
pub const STROKE_STRONG: Color = color(0x363c43, 0xd3d8df);

// Text.
pub const TEXT: Color = color(0xf4f6f8, 0x0f1419);
pub const SECONDARY: Color = color(0xb6bfcb, 0x4a5462);
pub const MUTED: Color = color(0x949ba6, 0x6b7481);
pub const PLACEHOLDER: Color = color(0x81888f, 0x9aa2ad);

// Accent and states.
pub const ACCENT: Color = color(0x5db7ef, 0x3390ec);
pub const LINK: Color = color(0x6cb6f2, 0x2481d7);
/// The selected chat: a deep tint in dark mode, solid accent in light.
pub const SELECTED: Color = color(0x2f5280, 0x3390ec);
/// Text and meta on SELECTED.
pub const SELECTED_TEXT: Color = color(0xe7eef7, 0xffffff);
pub const SELECTED_META: Color = color(0xbcd1ea, 0xd6e8fc);
pub const BADGE: Color = color(0x5b9ff7, 0x3390ec);
pub const BADGE_MUTED: Color = color(0x4a5059, 0xb3bac4);
/// Primary button states.
pub const PRIMARY_HOVER: Color = color(0x6fabf8, 0x4a9df0);
pub const PRIMARY_ACTIVE: Color = color(0x4f8fe0, 0x2a7fd6);
pub const DANGER: Color = color(0xf06b62, 0xe0453a);
/// The "online" dot on avatars.
pub const ONLINE: Color = color(0x4cd36a, 0x2fbf55);
/// Hover on destructive menu items.
pub const DANGER_HOVER: Color = color(0x3a2526, 0xfde9e7);

// Messages.
/// Behind the message history and composer: in light mode a soft tint, so
/// white bubbles and the white composer stand off it.
pub const HISTORY: Color = color(0x1a1e24, 0xecf0f5);
pub const BUBBLE_IN: Color = color(0x2c3137, 0xffffff);
pub const BUBBLE_OUT: Color = color(0x305483, 0xdcecff);
pub const META_IN: Color = color(0x9098a2, 0x8a929d);
pub const META_OUT: Color = color(0xacbfd5, 0x5683b0);
pub const CHECK_OUT: Color = color(0x87bbe9, 0x3390ec);
/// Accents inside outgoing bubbles: quote bars, chosen reactions, poll bars.
pub const OUT_ACCENT: Color = color(0xd4e8fc, 0x2f7fd6);
pub const OUT_LINK: Color = color(0xa9d3ff, 0x1e6cc4);
pub const OUT_SOFT_TEXT: Color = color(0xe4ecf2, 0x2a3846);
pub const OUT_FILE: Color = color(0x5b8bc4, 0x3390ec);
/// Date and event pills in the history.
pub const SERVICE: Color = color(0x272c32, 0xdfe5ec);
pub const SERVICE_TEXT: Color = color(0xd7dde7, 0x4a5462);
/// Code text and spoiler cover inside messages.
pub const CODE: Color = color(0x9ccfff, 0x1e6cc4);
pub const SPOILER: Color = color(0x3a4a5c, 0xc4cbd4);
pub const WHITE: Color = color(0xffffff, 0xffffff);

/// The message field.
pub const COMPOSER: Color = color(0x2c3137, 0xffffff);

// Role aliases.
pub const BORDER: Color = STROKE;
pub const DRAFT: Color = DANGER;

// Type scale.
/// Captions, key caps, section labels, badges.
pub const TEXT_XS: f32 = 11.5;
/// Meta: times, statuses, hints, secondary lines.
pub const TEXT_SM: f32 = 12.5;
/// Body: messages, previews, inputs, menu items.
pub const TEXT_MD: f32 = 14.;
/// Names and titles.
pub const TEXT_LG: f32 = 15.;
/// Headings: profile name, login title.
pub const TEXT_XL: f32 = 20.;

// Radii. Nested shapes are concentric: an inner corner's radius is the
// outer radius minus the gap between them, so the gap stays even around
// the curve (the rule macOS itself follows).
/// Key caps, quotes, small inner shapes.
pub const RADIUS_SM: f32 = 6.;
/// Controls: fields, buttons, list rows; menu items (RADIUS_LG cards with
/// a 6pt inset: 16 - 6).
pub const RADIUS_MD: f32 = 10.;
/// Containers: cards, floating layers.
pub const RADIUS_LG: f32 = 16.;
/// Gap between the window edge and panels, and between panels.
pub const INSET: f32 = 8.;
/// Main panels. macOS draws the window corner at a fixed 10pt (Sonoma,
/// Sequoia), which leaves only 2pt for a concentric panel at an 8pt inset;
/// panels stay properly round instead, close to the window's curve, and
/// everything nested inside them is concentric with them.
pub const RADIUS_PANEL: f32 = 12.;
/// Message bubbles: open corners, corners joining the next bubble from the
/// same sender, and the tail corner closing a run.
pub const RADIUS_BUBBLE: f32 = RADIUS_LG;
pub const RADIUS_BUBBLE_JOINED: f32 = RADIUS_SM;
pub const RADIUS_BUBBLE_TAIL: f32 = 4.;
/// Inset of content from a panel's edges. Every header, bar and list lines
/// up on it; trailing icon buttons sit `GUTTER - 8` in, so their 16px
/// glyphs end on the same line.
pub const GUTTER: f32 = 16.;
/// Height of panel headers (chat header, side panels).
pub const HEADER: f32 = 56.;

// Control heights.
pub const CONTROL_SM: f32 = 28.;
pub const CONTROL_MD: f32 = 36.;
pub const CONTROL_LG: f32 = 44.;

// Icons.
pub const ICON_SM: f32 = 14.;
pub const ICON_MD: f32 = 16.;
pub const ICON_LG: f32 = 18.;

// Avatars.
pub const AVATAR_SM: f32 = 32.;
pub const AVATAR_MD: f32 = 40.;
pub const AVATAR_LG: f32 = 48.;
pub const AVATAR_XL: f32 = 96.;

/// Telegram's seven peer colors: red, orange, violet, green, cyan, blue,
/// pink. Names (darker in light mode, to read on light bubbles), then
/// avatar fills.
const PEER_NAMES: [Color; 7] = [
    color(0xfb6169, 0xd1453b),
    color(0xfaa357, 0xc96f0c),
    color(0xb48bf2, 0x8550d6),
    color(0x85de85, 0x2f9a26),
    color(0x62d4e3, 0x1592ad),
    color(0x65bdf3, 0x2a7fd0),
    color(0xff5694, 0xcf3f7c),
];
const PEER_AVATARS: [u32; 7] = [
    0xff845e, 0xfebb5b, 0xb694f9, 0x9ad164, 0x5bcbe3, 0x5caffa, 0xff8aac,
];

pub fn peer_index(id: i64) -> usize {
    // Telegram derives the color from the id; any stable spread is fine here.
    (id.unsigned_abs() % 7) as usize
}
pub fn peer_name_color(id: i64) -> Color {
    PEER_NAMES[peer_index(id)]
}
pub fn peer_avatar_color(id: i64) -> u32 {
    PEER_AVATARS[peer_index(id)]
}

// ---------------------------------------------------------------------------
// Light/dark: the appearance `Paint` reads; set from the window's.
// ---------------------------------------------------------------------------

static LIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_light(light: bool) {
    LIGHT.store(light, std::sync::atomic::Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Dims the app behind the palette and dialogs: a light slate veil in light
/// mode, where flat black turns everything muddy.
pub fn scrim() -> Hsla {
    if is_light() {
        hsla_alpha(0x1b2533, 0x2e)
    } else {
        hsla_alpha(0x000000, 0x66)
    }
}

/// Lifts bubbles and the composer off the light history tint; dark mode
/// separates them by tone alone.
pub fn lift() -> Vec<gpui_kit::BoxShadow> {
    if !is_light() {
        return Vec::new();
    }
    vec![gpui_kit::BoxShadow {
        color: gpui_kit::hsla(215. / 360., 0.25, 0.25, 0.10),
        offset: gpui_kit::point(px(0.), px(1.)),
        blur_radius: px(2.),
        spread_radius: px(0.),
        inset: false,
    }]
}

pub fn hsla(color: impl Paint) -> Hsla {
    rgb(color.rgb()).into()
}
pub fn hsla_alpha(color: impl Paint, alpha: u8) -> Hsla {
    rgba((color.rgb() << 8) | u32::from(alpha)).into()
}

pub fn local(timestamp: i32) -> Option<DateTime<Local>> {
    Local.timestamp_opt(i64::from(timestamp), 0).single()
}

/// Time shown inside bubbles.
pub fn clock(timestamp: i32) -> String {
    local(timestamp).map_or_else(String::new, |t| t.format("%H:%M").to_string())
}

/// Compact date for the chat list: time today, weekday this week, date otherwise.
pub fn list_date(timestamp: i32) -> String {
    let Some(time) = local(timestamp) else {
        return String::new();
    };
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(time.date_naive())
        .num_days();
    if days <= 0 {
        time.format("%H:%M").to_string()
    } else if days < 7 {
        time.format("%a").to_string()
    } else if time.year() == now.year() {
        time.format("%b %-d").to_string()
    } else {
        time.format("%d.%m.%y").to_string()
    }
}

/// Day separator label in the history.
pub fn day_label(timestamp: i32) -> String {
    let Some(time) = local(timestamp) else {
        return String::new();
    };
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(time.date_naive())
        .num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ if time.year() == now.year() => time.format("%B %-d").to_string(),
        _ => time.format("%B %-d, %Y").to_string(),
    }
}

pub fn same_day(a: i32, b: i32) -> bool {
    local(a)
        .zip(local(b))
        .is_some_and(|(a, b)| a.date_naive() == b.date_naive())
}

pub fn user_status(status: &UserStatus) -> (String, bool) {
    match status {
        UserStatus::Online(_) => ("online".into(), true),
        UserStatus::Offline(offline) => {
            let Some(time) = local(offline.was_online) else {
                return ("last seen a long time ago".into(), false);
            };
            let now = Local::now();
            let minutes = now.signed_duration_since(time).num_minutes();
            let text = if minutes < 1 {
                "last seen just now".into()
            } else if minutes < 60 {
                format!(
                    "last seen {minutes} minute{} ago",
                    if minutes == 1 { "" } else { "s" }
                )
            } else if same_day(offline.was_online, now.timestamp() as i32) {
                format!("last seen at {}", time.format("%H:%M"))
            } else if now
                .date_naive()
                .signed_duration_since(time.date_naive())
                .num_days()
                == 1
            {
                format!("last seen yesterday at {}", time.format("%H:%M"))
            } else {
                format!("last seen {}", time.format("%b %-d"))
            };
            (text, false)
        }
        UserStatus::Recently(_) => ("last seen recently".into(), false),
        UserStatus::LastWeek(_) => ("last seen within a week".into(), false),
        UserStatus::LastMonth(_) => ("last seen within a month".into(), false),
        UserStatus::Empty => ("last seen a long time ago".into(), false),
    }
}

pub fn count_label(count: i32, one: &str, many: &str) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    format!("{grouped} {}", if count == 1 { one } else { many })
}

pub fn file_size(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes < 1024.0 {
        format!("{bytes:.0} B")
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KB", bytes / 1024.0)
    } else if bytes < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", bytes / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB", bytes / 1024.0 / 1024.0 / 1024.0)
    }
}

pub fn duration(seconds: i32) -> String {
    let seconds = seconds.max(0);
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

pub fn initials(title: &str) -> String {
    let letters: String = title
        .split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if letters.is_empty() {
        title.chars().take(1).collect()
    } else {
        letters
    }
}

/// A link inside rich text: byte range plus target.
pub struct Link {
    pub range: Range<usize>,
    pub url: String,
}

pub struct RichText {
    pub text: String,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
    pub code: Vec<Range<usize>>,
    pub links: Vec<Link>,
}

/// Converts Telegram entities (UTF-16 offsets, possibly overlapping) into
/// sorted, non-overlapping GPUI highlights.
pub fn rich_text(formatted: &FormattedText, link_color: Color) -> RichText {
    let text = &formatted.text;
    // Map UTF-16 offsets to byte offsets.
    let mut offsets = Vec::with_capacity(text.len() + 1);
    for (byte, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            offsets.push(byte);
        }
    }
    offsets.push(text.len());
    let byte = |utf16: i32| -> usize {
        usize::try_from(utf16)
            .ok()
            .and_then(|i| offsets.get(i).copied())
            .unwrap_or(text.len())
    };

    #[derive(Clone, Copy, Default)]
    struct Flags {
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
        link: bool,
        code: bool,
        spoiler: bool,
    }
    let mut spans: Vec<(Range<usize>, Flags)> = Vec::new();
    let mut links = Vec::new();
    for entity in &formatted.entities {
        let range = byte(entity.offset)..byte(entity.offset.saturating_add(entity.length));
        if range.is_empty() {
            continue;
        }
        let mut flags = Flags::default();
        match &entity.r#type {
            TextEntityType::Bold => flags.bold = true,
            TextEntityType::Italic => flags.italic = true,
            TextEntityType::Underline => flags.underline = true,
            TextEntityType::Strikethrough => flags.strike = true,
            TextEntityType::Spoiler => flags.spoiler = true,
            TextEntityType::Code | TextEntityType::Pre | TextEntityType::PreCode(_) => {
                flags.code = true
            }
            TextEntityType::Url => {
                let raw = &text[range.clone()];
                let url = if raw.contains("://") {
                    raw.to_owned()
                } else {
                    format!("https://{raw}")
                };
                links.push(Link {
                    range: range.clone(),
                    url,
                });
                flags.link = true;
            }
            TextEntityType::TextUrl(link) => {
                links.push(Link {
                    range: range.clone(),
                    url: link.url.clone(),
                });
                flags.link = true;
            }
            TextEntityType::EmailAddress => {
                links.push(Link {
                    range: range.clone(),
                    url: format!("mailto:{}", &text[range.clone()]),
                });
                flags.link = true;
            }
            TextEntityType::Mention => {
                links.push(Link {
                    range: range.clone(),
                    url: format!(
                        "https://t.me/{}",
                        text[range.clone()].trim_start_matches('@')
                    ),
                });
                flags.link = true;
            }
            TextEntityType::Hashtag
            | TextEntityType::Cashtag
            | TextEntityType::BotCommand
            | TextEntityType::MentionName(_)
            | TextEntityType::PhoneNumber => flags.link = true,
            _ => continue,
        }
        spans.push((range, flags));
    }
    links.sort_by_key(|link| link.range.start);
    links.dedup_by(|a, b| a.range.start < b.range.end);

    let mut bounds: Vec<usize> = spans
        .iter()
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut highlights = Vec::new();
    let mut code = Vec::new();
    for pair in bounds.windows(2) {
        let range = pair[0]..pair[1];
        let mut flags = Flags::default();
        for (span, f) in &spans {
            if span.start <= range.start && range.end <= span.end {
                flags.bold |= f.bold;
                flags.italic |= f.italic;
                flags.underline |= f.underline;
                flags.strike |= f.strike;
                flags.link |= f.link;
                flags.code |= f.code;
                flags.spoiler |= f.spoiler;
            }
        }
        let mut style = HighlightStyle::default();
        let mut styled = false;
        if flags.bold {
            style.font_weight = Some(FontWeight::SEMIBOLD);
            styled = true;
        }
        if flags.italic {
            style.font_style = Some(FontStyle::Italic);
            styled = true;
        }
        if flags.underline {
            style.underline = Some(UnderlineStyle {
                thickness: px(1.),
                ..Default::default()
            });
            styled = true;
        }
        if flags.strike {
            style.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                ..Default::default()
            });
            styled = true;
        }
        if flags.link {
            style.color = Some(hsla(link_color));
            styled = true;
        }
        if flags.code {
            style.color = Some(hsla(CODE));
            style.background_color = Some(hsla_alpha(CODE, 0x1f));
            code.push(range.clone());
            styled = true;
        }
        if flags.spoiler {
            style.background_color = Some(hsla(SPOILER));
            style.color = Some(hsla(SPOILER));
            styled = true;
        }
        if styled {
            highlights.push((range, style));
        }
    }
    RichText {
        text: text.clone(),
        highlights,
        code,
        links,
    }
}

/// Turns entities back into the markdown the composer accepts, so editing a
/// formatted message keeps its formatting.
pub fn to_markdown(formatted: &FormattedText) -> String {
    let text = &formatted.text;
    let mut offsets = Vec::with_capacity(text.len() + 1);
    for (byte, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            offsets.push(byte);
        }
    }
    offsets.push(text.len());
    let byte = |utf16: i32| {
        usize::try_from(utf16)
            .ok()
            .and_then(|i| offsets.get(i).copied())
            .unwrap_or(text.len())
    };
    // (position, closing, entity length, marker): closings sort before
    // openings at the same spot; outer entities open first and close last.
    let mut marks: Vec<(usize, bool, i32, String)> = Vec::new();
    for entity in &formatted.entities {
        let (open, close) = match &entity.r#type {
            TextEntityType::Bold => ("**".to_owned(), "**".to_owned()),
            TextEntityType::Italic => ("__".to_owned(), "__".to_owned()),
            TextEntityType::Strikethrough => ("~~".to_owned(), "~~".to_owned()),
            TextEntityType::Spoiler => ("||".to_owned(), "||".to_owned()),
            TextEntityType::Code => ("`".to_owned(), "`".to_owned()),
            TextEntityType::Pre | TextEntityType::PreCode(_) => {
                ("```\n".to_owned(), "\n```".to_owned())
            }
            TextEntityType::TextUrl(link) => ("[".to_owned(), format!("]({})", link.url)),
            _ => continue,
        };
        let start = byte(entity.offset);
        let end = byte(entity.offset.saturating_add(entity.length));
        marks.push((start, false, -entity.length, open));
        marks.push((end, true, entity.length, close));
    }
    marks.sort_by_key(|a| (a.0, !a.1, a.2));
    let mut out = String::with_capacity(text.len() + marks.len() * 2);
    let mut last = 0;
    for (position, _, _, marker) in marks {
        out.push_str(&text[last..position]);
        out.push_str(&marker);
        last = position;
    }
    out.push_str(&text[last..]);
    out
}

/// Decodes TDLib's base64 minithumbnail payload.
pub fn base64(data: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in data.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// Switches the palette and restyles the kit's controls (menus, dialogs,
/// inputs) to match.
pub fn apply_theme(light: bool, cx: &mut gpui_kit::App) {
    use gpui_kit::component::{Theme, ThemeMode};
    set_light(light);
    Theme::change(
        if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        theme.font_size = px(TEXT_MD);
        theme.radius = px(RADIUS_MD);
        theme.radius_lg = px(RADIUS_LG);
        theme.background = hsla(BASE);
        theme.foreground = hsla(TEXT);
        theme.border = hsla(STROKE_STRONG);
        theme.input = hsla(STROKE_STRONG);
        theme.muted = hsla(RAISED);
        theme.muted_foreground = hsla(MUTED);
        theme.accent = hsla(HOVER_RAISED);
        theme.accent_foreground = hsla(TEXT);
        theme.popover = hsla(OVERLAY);
        theme.popover_foreground = hsla(TEXT);
        theme.primary = hsla(BADGE);
        theme.primary_hover = hsla(PRIMARY_HOVER);
        theme.primary_active = hsla(PRIMARY_ACTIVE);
        theme.primary_foreground = hsla(WHITE);
        theme.button_primary = hsla(BADGE);
        theme.button_primary_hover = hsla(PRIMARY_HOVER);
        theme.button_primary_active = hsla(PRIMARY_ACTIVE);
        theme.button_primary_foreground = hsla(WHITE);
        theme.secondary = hsla(RAISED);
        theme.secondary_hover = hsla(HOVER_RAISED);
        theme.secondary_active = hsla(HOVER_RAISED);
        theme.secondary_foreground = hsla(TEXT);
        theme.list_hover = hsla(HOVER_RAISED);
        theme.list_active = hsla(SELECTED);
        theme.tab_bar = hsla(BASE);
        theme.tab = hsla(BASE);
        theme.tab_active = hsla(BASE);
        theme.tab_foreground = hsla(MUTED);
        theme.tab_active_foreground = hsla(ACCENT);
        theme.link = hsla(LINK);
        theme.selection = hsla_alpha(ACCENT, 0x55);
        theme.caret = hsla(ACCENT);
        theme.ring = hsla_alpha(ACCENT, 0x80);
        theme.overlay = scrim();
        let thumb = if light { 0x000000 } else { 0xffffff };
        theme.scrollbar_thumb = hsla_alpha(thumb, 0x30);
        theme.scrollbar_thumb_hover = hsla_alpha(thumb, 0x50);
        theme.danger = hsla(DANGER);
        theme.focus_ring = false;
    });
}
