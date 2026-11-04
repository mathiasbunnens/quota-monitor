//! Interactive AppKit fixture for the native menu's live refresh
//! (`--menu-tracking-fixture <dir>`). Synthetic data only: no Tauri, no
//! preferences, no quota provider, no notification, no updater.
//!
//! It pops the production menu (`make_native_quota_menu`) up for real, so
//! AppKit runs its menu-tracking loop, highlights « Réglages… » then
//! « Vérifier les mises à jour… » with keyboard and mouse-moved events posted
//! to this process only (the visible cursor does not move), and refreshes the
//! quota panel during tracking: same-height refresh (countdown tick), height
//! change, bursts while dwelling. Two variants per appearance:
//! `legacy` = `setView` as up to 644cca6, `production` = `refresh_open_panel`.
//!
//! After each step it locates AppKit's selection background
//! (`NSRootMenuWindowBackgroundView`, the blue rounded highlight) in the menu
//! window and reports whether it lies over the quota panel row (the stray
//! band) or over the highlighted action's row. It also writes a PNG drawn by
//! the window's own views (`cacheDisplayInRect`; glass material omitted) and
//! the view tree.
use super::*;
use block2::RcBlock;
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplicationActivationPolicy,
    NSBitmapImageFileType, NSWindow,
};
use objc2_foundation::{NSDictionary, NSRunLoop, NSRunLoopCommonModes, NSTimer};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::fmt::Write as _;
use std::rc::Rc;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        kind: u32,
        location: CGPoint,
        button: u32,
    ) -> *mut c_void;
    fn CGEventPostToPid(pid: i32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

const KEY_DOWN_ARROW: u16 = 125;
const MOUSE_MOVED: u32 = 5;
const SELECTION: &str = "NSRootMenuWindowBackgroundView";

fn post_key(key: u16) {
    unsafe {
        for down in [true, false] {
            let event = CGEventCreateKeyboardEvent(std::ptr::null(), key, down);
            CGEventPostToPid(std::process::id() as i32, event);
            CFRelease(event);
        }
    }
}

/// `point` in global top-left coordinates (CoreGraphics).
fn post_mouse_moved(point: CGPoint) {
    unsafe {
        let event = CGEventCreateMouseEvent(std::ptr::null(), MOUSE_MOVED, point, 0);
        CGEventPostToPid(std::process::id() as i32, event);
        CFRelease(event);
    }
}

fn class_name(object: &AnyObject) -> String {
    object.class().name().to_string_lossy().into_owned()
}

type Node = (usize, String, NSRect, bool);

/// Every view under `view`, with its frame in `root` coordinates.
fn walk(root: &NSView, view: &NSView, depth: usize, out: &mut Vec<Node>) {
    let rect = view.convertRect_toView(view.bounds(), Some(root));
    out.push((depth, class_name(view), rect, view.isHidden()));
    if depth < 12 {
        for sub in view.subviews().iter() {
            walk(root, &sub, depth + 1, out);
        }
    }
}

fn menu_windows(mtm: MainThreadMarker) -> Vec<Retained<NSWindow>> {
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .filter(|w| w.isVisible() && class_name(w).contains("Menu"))
        .collect()
}

fn overlap(a: NSRect, b: NSRect) -> f64 {
    let top = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    (top - a.origin.y.max(b.origin.y)).max(0.0)
}

fn span(r: &NSRect) -> String {
    format!("y{:.0}-{:.0}", r.origin.y, r.origin.y + r.size.height)
}

fn save_png(view: &NSView, path: &str) {
    let Some(bitmap) = view.bitmapImageRepForCachingDisplayInRect(view.bounds()) else {
        return;
    };
    view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &bitmap);
    if let Some(png) = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    } {
        png.writeToFile_atomically(&NSString::from_str(path), true);
    }
}

fn scenes(mtm: MainThreadMarker) -> Vec<Retained<NSView>> {
    let soon = (Local::now() + chrono::Duration::minutes(95)).to_rfc3339();
    let week = (Local::now() + chrono::Duration::days(4)).to_rfc3339();
    let codex_rows = vec![
        demo_payload("Limite 5 heures", "five-hour", 63, &soon, Some(300)),
        demo_payload("Limite hebdomadaire", "weekly", 85, &week, Some(10080)),
    ];
    let claude = claude_panel(&claude::usage_snapshot(
        &serde_json::json!({"subscription_type":"pro", "rate_limits":{
            "five_hour":{"utilization":42,"resets_at":soon},
            "seven_day":{"utilization":20,"resets_at":week}
        }}),
        Local::now().timestamp(),
    ));
    let both = vec![
        codex_panel(&codex_rows, "ready", "", Some("plus".into())),
        claude.clone(),
    ];
    let waiting = vec![
        codex_panel(&codex_rows, "ready", "", Some("plus".into())),
        PanelProvider {
            rows: vec![],
            note: Some(("Lecture via Claude Code…".into(), Tone::Warning)),
            ..claude
        },
    ];
    let view = |providers: &[PanelProvider]| {
        make_panel_view(mtm, &panel_header(providers, Some(Local::now())), providers)
    };
    // 0 initial, 1 same height (tick/refresh), 2 other height, 3 back.
    vec![view(&both), view(&both), view(&waiting), view(&both)]
}

#[derive(Clone, Copy)]
enum Step {
    Key,
    HoverItem(usize),
    Update(usize),
    Capture(&'static str),
    Close,
}

/// Inspects the menu window after a step; returns whether a stray band is
/// drawn over the panel.
fn inspect(
    mtm: MainThreadMarker,
    menu: &NSMenu,
    panel: &NSMenuItem,
    name: &str,
    dir: &str,
    report: &mut String,
) -> bool {
    let highlighted = menu
        .highlightedItem()
        .map(|i| i.title().to_string())
        .unwrap_or_else(|| "<none>".into());
    let panel_h = panel.view().map(|v| v.frame().size.height).unwrap_or(0.0);
    let mut stray = false;
    for window in menu_windows(mtm) {
        let Some(content) = window.contentView() else {
            continue;
        };
        let root = unsafe { content.superview() }.unwrap_or(content);
        let mut nodes = Vec::new();
        walk(&root, &root, 0, &mut nodes);
        // Panel row: the custom item view as tall as the panel.
        let panel_rect = nodes
            .iter()
            .find(|(_, c, r, _)| {
                c == "NSCustomMenuItemView" && (r.size.height - panel_h).abs() < 0.5
            })
            .map(|n| n.2);
        let selections: Vec<NSRect> = nodes
            .iter()
            .filter(|(_, c, _, hidden)| c == SELECTION && !hidden)
            .map(|n| n.2)
            .collect();
        let band = panel_rect.is_some_and(|p| selections.iter().any(|s| overlap(*s, p) > 2.0));
        // Rows top to bottom = menu items in order; the selection must sit on
        // the highlighted item's own row.
        let mut rows: Vec<NSRect> = nodes
            .iter()
            .filter(|(_, c, _, _)| c == "NSTableRowView")
            .map(|n| n.2)
            .collect();
        rows.sort_by(|a, b| (b.origin.y + b.size.height).total_cmp(&(a.origin.y + a.size.height)));
        let expected = menu
            .highlightedItem()
            .and_then(|item| rows.get(menu.indexOfItem(&item) as usize).copied());
        let misplaced = match expected {
            Some(row) => !selections
                .iter()
                .any(|s| overlap(*s, row) > row.size.height - 1.0),
            None => false,
        } || selections.len() > 1;
        let verdict = if band {
            "STRAY_BAND_OVER_PANEL"
        } else if misplaced {
            "SELECTION_OFF_ITS_ROW"
        } else {
            "ok"
        };
        stray |= band || misplaced;
        let _ = writeln!(
            report,
            "{name}: highlighted={highlighted:?} row={} panel_row={} selection=[{}] -> {verdict}",
            expected.as_ref().map(span).unwrap_or("-".into()),
            panel_rect.as_ref().map(span).unwrap_or("?".into()),
            selections.iter().map(span).collect::<Vec<_>>().join(","),
        );
        let mut tree = String::new();
        for (depth, class, r, hidden) in &nodes {
            let _ = writeln!(
                tree,
                "{:indent$}{class} hidden={hidden} y={:.1} h={:.1} x={:.1} w={:.1}",
                "",
                r.origin.y,
                r.size.height,
                r.origin.x,
                r.size.width,
                indent = depth * 2
            );
        }
        std::fs::write(format!("{dir}/{name}.views.txt"), tree).unwrap();
        save_png(&root, &format!("{dir}/{name}.png"));
    }
    stray
}

/// Returns `false` when a stray band was observed in the production variant.
pub fn menu_tracking_fixture(directory: &str) -> bool {
    use Step::*;
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
    std::fs::create_dir_all(directory).unwrap();
    let target = QuotaMenuTarget::new(mtm);
    let report = Rc::new(RefCell::new(String::new()));
    let production_bands = Rc::new(Cell::new(0usize));
    // Keyboard: Réglages, dwell with refreshes, then Vérifier les MAJ.
    // Mouse: the same two items reached with posted mouse-moved events.
    let plan = [
        Key,
        Capture("01-reglages"),
        Update(1),
        Capture("02-reglages-tick"),
        Update(2),
        Capture("03-reglages-height"),
        Update(3),
        Update(1),
        Update(3),
        Capture("04-reglages-burst"),
        Key,
        Capture("05-maj"),
        Update(2),
        Capture("06-maj-height"),
        Update(3),
        Capture("07-maj-back"),
        HoverItem(2),
        Capture("08-hover-reglages"),
        Update(1),
        Capture("09-hover-reglages-tick"),
        Update(2),
        Capture("10-hover-reglages-height"),
        HoverItem(3),
        Update(3),
        Capture("11-hover-maj-back"),
        Close,
    ];
    for (mode, appearance) in [
        ("light", unsafe { NSAppearanceNameAqua }),
        ("dark", unsafe { NSAppearanceNameDarkAqua }),
    ] {
        for legacy in [true, false] {
            let variant = if legacy { "legacy" } else { "production" };
            app.setAppearance(NSAppearance::appearanceNamed(appearance).as_deref());
            let views = scenes(mtm);
            let (menu, panel) = make_native_quota_menu(mtm, &target, &views[0], None, "fixture");
            let index = Cell::new(0usize);
            let dir = format!("{directory}/{mode}-{variant}");
            std::fs::create_dir_all(&dir).unwrap();
            let _ = writeln!(report.borrow_mut(), "== {mode} {variant}");
            let (menu2, panel2, report2, bands) = (
                menu.clone(),
                panel.clone(),
                report.clone(),
                production_bands.clone(),
            );
            let block = RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
                let mtm = MainThreadMarker::new().unwrap();
                let step = plan.get(index.get()).copied().unwrap_or(Close);
                index.set(index.get() + 1);
                match step {
                    Key => post_key(KEY_DOWN_ARROW),
                    HoverItem(item) => {
                        // Item centre in CG (top-left) coordinates: panel row,
                        // separator ≈ 11 pt, then 24 pt actions.
                        if let Some(window) = menu_windows(mtm).first() {
                            let f = window.frame();
                            let screen_h = window
                                .screen()
                                .map(|s| s.frame().size.height)
                                .unwrap_or(0.0);
                            let panel_h =
                                panel2.view().map(|v| v.frame().size.height).unwrap_or(0.0);
                            let y_in = 5.0 + panel_h + 11.0 + (item as f64 - 2.0) * 24.0 + 12.0;
                            let top = screen_h - (f.origin.y + f.size.height);
                            post_mouse_moved(CGPoint {
                                x: f.origin.x + f.size.width / 2.0,
                                y: top + y_in,
                            });
                        }
                    }
                    Update(scene) => {
                        if legacy {
                            panel2.setView(Some(&views[scene]));
                        } else if !refresh_open_panel(&panel2, &views[scene]) {
                            let _ = writeln!(
                                report2.borrow_mut(),
                                "  update {scene}: height change deferred (action highlighted)"
                            );
                        }
                    }
                    Capture(name) => {
                        let stray =
                            inspect(mtm, &menu2, &panel2, name, &dir, &mut report2.borrow_mut());
                        if stray && !legacy {
                            bands.set(bands.get() + 1);
                        }
                    }
                    Close => {
                        unsafe { timer.as_ref() }.invalidate();
                        menu2.cancelTracking();
                    }
                }
            });
            let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(0.3, true, &block) };
            unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
            // Top-right of the main screen, like a status-item menu.
            let screen = objc2_app_kit::NSScreen::mainScreen(mtm).expect("screen");
            let visible = screen.visibleFrame();
            let at = NSPoint::new(
                visible.origin.x + visible.size.width - PANEL_WIDTH - 40.0,
                visible.origin.y + visible.size.height - 4.0,
            );
            let _ = menu.popUpMenuPositioningItem_atLocation_inView(None, at, None);
            timer.invalidate();
        }
    }
    let report = report.borrow();
    std::fs::write(format!("{directory}/report.txt"), report.as_str()).unwrap();
    print!("{report}");
    let ok = production_bands.get() == 0;
    println!(
        "MENU_TRACKING_FIXTURE {} (production stray bands: {})",
        if ok { "PASS" } else { "FAIL" },
        production_bands.get()
    );
    ok
}
