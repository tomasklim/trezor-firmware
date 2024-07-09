use core::mem;

use crate::{
    strutil::{ShortString, TString},
    time::Duration,
    trezorhal::random,
    ui::{
        component::{
            base::ComponentExt, text::TextStyle, Child, Component, Event, EventCtx, Label, Maybe,
            Never, Pad, TimerToken,
        },
        display::Font,
        event::TouchEvent,
        geometry::{Alignment, Alignment2D, Grid, Insets, Offset, Rect},
        model_mercury::component::{
            button::{
                Button, ButtonContent,
                ButtonMsg::{self, Clicked},
            },
            theme,
        },
        shape::{self, Renderer},
    },
};

pub enum PinKeyboardMsg {
    Confirmed,
    Cancelled,
}

const MAX_LENGTH: usize = 50;
const MAX_VISIBLE_DOTS: usize = 18;
const MAX_VISIBLE_DIGITS: usize = 18;
const DIGIT_COUNT: usize = 10; // 0..10

const HEADER_PADDING_TOP: i16 = 4;
const HEADER_PADDING_SIDE: i16 = 2;
const HEADER_PADDING_BOTTOM: i16 = 4;

const HEADER_PADDING: Insets = Insets::new(
    HEADER_PADDING_TOP,
    HEADER_PADDING_SIDE,
    HEADER_PADDING_BOTTOM,
    HEADER_PADDING_SIDE,
);

pub struct PinKeyboard<'a> {
    allow_cancel: bool,
    major_prompt: Child<Label<'a>>,
    minor_prompt: Child<Label<'a>>,
    major_warning: Option<Child<Label<'a>>>,
    textbox: Child<PinDots>,
    textbox_pad: Pad,
    erase_btn: Child<Maybe<Button>>,
    cancel_btn: Child<Maybe<Button>>,
    confirm_btn: Child<Button>,
    digit_btns: [Child<Button>; DIGIT_COUNT],
    warning_timer: Option<TimerToken>,
}

impl<'a> PinKeyboard<'a> {
    pub fn new(
        major_prompt: TString<'a>,
        minor_prompt: TString<'a>,
        major_warning: Option<TString<'a>>,
        allow_cancel: bool,
    ) -> Self {
        // Control buttons.
        let erase_btn = Button::with_icon(theme::ICON_DELETE)
            .styled(theme::button_keyboard_erase())
            .with_long_press(theme::ERASE_HOLD_DURATION)
            .initially_enabled(false);
        let erase_btn = Maybe::hidden(theme::BG, erase_btn).into_child();

        let cancel_btn =
            Button::with_icon(theme::ICON_CLOSE).styled(theme::button_keyboard_cancel());
        let cancel_btn = Maybe::new(theme::BG, cancel_btn, allow_cancel).into_child();

        Self {
            allow_cancel,
            major_prompt: Label::left_aligned(major_prompt, theme::label_keyboard()).into_child(),
            minor_prompt: Label::right_aligned(minor_prompt, theme::label_keyboard_minor())
                .into_child(),
            major_warning: major_warning.map(|text| {
                Label::left_aligned(text, theme::label_keyboard_warning()).into_child()
            }),
            textbox: PinDots::new(theme::label_default()).into_child(),
            textbox_pad: Pad::with_background(theme::label_default().background_color),
            erase_btn,
            cancel_btn,
            confirm_btn: Button::with_icon(theme::ICON_SIMPLE_CHECKMARK24)
                .styled(theme::button_pin_confirm())
                .initially_enabled(false)
                .into_child(),
            digit_btns: Self::generate_digit_buttons(),
            warning_timer: None,
        }
    }

    fn generate_digit_buttons() -> [Child<Button>; DIGIT_COUNT] {
        // Generate a random sequence of digits from 0 to 9.
        let mut digits = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
        random::shuffle(&mut digits);
        digits
            .map(|c| Button::with_text(c.into()))
            .map(|b| {
                b.styled(theme::button_keyboard())
                    .with_text_align(Alignment::Center)
            })
            .map(Child::new)
    }

    fn pin_modified(&mut self, ctx: &mut EventCtx) {
        let is_full = self.textbox.inner().is_full();
        let is_empty = self.textbox.inner().is_empty();

        self.textbox_pad.clear();
        self.textbox.request_complete_repaint(ctx);

        if is_empty {
            self.major_prompt.request_complete_repaint(ctx);
            self.minor_prompt.request_complete_repaint(ctx);
            self.major_warning.request_complete_repaint(ctx);
        }

        let cancel_enabled = is_empty && self.allow_cancel;
        for btn in &mut self.digit_btns {
            btn.mutate(ctx, |ctx, btn| btn.enable_if(ctx, !is_full));
        }
        self.erase_btn.mutate(ctx, |ctx, btn| {
            btn.show_if(ctx, !is_empty);
            btn.inner_mut().enable_if(ctx, !is_empty);
        });
        self.cancel_btn.mutate(ctx, |ctx, btn| {
            btn.show_if(ctx, cancel_enabled);
            btn.inner_mut().enable_if(ctx, is_empty);
        });
        self.confirm_btn
            .mutate(ctx, |ctx, btn| btn.enable_if(ctx, !is_empty));
    }

    pub fn pin(&self) -> &str {
        self.textbox.inner().pin()
    }
}

impl Component for PinKeyboard<'_> {
    type Msg = PinKeyboardMsg;

    fn place(&mut self, bounds: Rect) -> Rect {
        // Prompts and PIN dots display.
        let (header, keypad) =
            bounds.split_bottom(4 * theme::PIN_BUTTON_HEIGHT + 3 * theme::BUTTON_SPACING);
        let prompt = header.inset(HEADER_PADDING);

        // Control buttons.
        let grid = Grid::new(keypad, 4, 3).with_spacing(theme::BUTTON_SPACING);

        // Prompts and PIN dots display.
        self.textbox_pad.place(header);
        self.textbox.place(header);
        self.major_prompt.place(prompt);
        self.minor_prompt.place(prompt);
        self.major_warning.as_mut().map(|c| c.place(prompt));

        // Control buttons.
        let erase_cancel_area = grid.row_col(3, 0);
        self.erase_btn.place(erase_cancel_area);
        self.cancel_btn.place(erase_cancel_area);
        self.confirm_btn.place(grid.row_col(3, 2));

        // Digit buttons.
        for (i, btn) in self.digit_btns.iter_mut().enumerate() {
            // Assign the digits to buttons on a 4x3 grid, starting from the first row.
            let area = grid.cell(if i < 9 {
                i
            } else {
                // For the last key (the "0" position) we skip one cell.
                i + 1
            });
            btn.place(area);
        }

        bounds
    }

    fn event(&mut self, ctx: &mut EventCtx, event: Event) -> Option<Self::Msg> {
        match event {
            // Set up timer to switch off warning prompt.
            Event::Attach(_) if self.major_warning.is_some() => {
                self.warning_timer = Some(ctx.request_timer(Duration::from_secs(2)));
            }
            // Hide warning, show major prompt.
            Event::Timer(token) if Some(token) == self.warning_timer => {
                self.major_warning = None;
                self.textbox_pad.clear();
                self.minor_prompt.request_complete_repaint(ctx);
                ctx.request_paint();
            }
            _ => {}
        }

        self.textbox.event(ctx, event);
        if let Some(Clicked) = self.confirm_btn.event(ctx, event) {
            return Some(PinKeyboardMsg::Confirmed);
        }
        if let Some(Clicked) = self.cancel_btn.event(ctx, event) {
            return Some(PinKeyboardMsg::Cancelled);
        }
        match self.erase_btn.event(ctx, event) {
            Some(ButtonMsg::Clicked) => {
                self.textbox.mutate(ctx, |ctx, t| t.pop(ctx));
                self.pin_modified(ctx);
                return None;
            }
            Some(ButtonMsg::LongPressed) => {
                self.textbox.mutate(ctx, |ctx, t| t.clear(ctx));
                self.pin_modified(ctx);
                return None;
            }
            _ => {}
        }
        for btn in &mut self.digit_btns {
            if let Some(Clicked) = btn.event(ctx, event) {
                if let ButtonContent::Text(text) = btn.inner().content() {
                    text.map(|text| {
                        self.textbox.mutate(ctx, |ctx, t| t.push(ctx, text));
                    });
                    self.pin_modified(ctx);
                    return None;
                }
            }
        }
        None
    }

    fn paint(&mut self) {
        todo!("remove when ui-t3t1 done");
    }

    fn render<'s>(&'s self, target: &mut impl Renderer<'s>) {
        self.erase_btn.render(target);
        self.textbox_pad.render(target);

        if self.textbox.inner().is_empty() {
            if let Some(ref w) = self.major_warning {
                w.render(target);
            } else {
                self.major_prompt.render(target);
            }
            self.minor_prompt.render(target);
            self.cancel_btn.render(target);
        } else {
            self.textbox.render(target);
        }

        self.confirm_btn.render(target);

        for btn in &self.digit_btns {
            btn.render(target);
        }
    }
}

struct PinDots {
    area: Rect,
    pad: Pad,
    style: TextStyle,
    digits: ShortString,
    display_digits: bool,
}

impl PinDots {
    const DOT: i16 = 6;
    const PADDING: i16 = 7;
    const TWITCH: i16 = 4;

    fn new(style: TextStyle) -> Self {
        Self {
            area: Rect::zero(),
            pad: Pad::with_background(style.background_color),
            style,
            digits: ShortString::new(),
            display_digits: false,
        }
    }

    fn size(&self) -> Offset {
        let ndots = self.digits.len().min(MAX_VISIBLE_DOTS);
        let mut width = Self::DOT * (ndots as i16);
        width += Self::PADDING * (ndots.saturating_sub(1) as i16);
        Offset::new(width, Self::DOT)
    }

    fn is_empty(&self) -> bool {
        self.digits.is_empty()
    }

    fn is_full(&self) -> bool {
        self.digits.len() >= MAX_LENGTH
    }

    fn clear(&mut self, ctx: &mut EventCtx) {
        self.digits.clear();
        ctx.request_paint()
    }

    fn push(&mut self, ctx: &mut EventCtx, text: &str) {
        if self.digits.push_str(text).is_err() {
            // `self.pin` is full and wasn't able to accept all of
            // `text`. Should not happen.
        };
        ctx.request_paint()
    }

    fn pop(&mut self, ctx: &mut EventCtx) {
        if self.digits.pop().is_some() {
            ctx.request_paint()
        }
    }

    fn pin(&self) -> &str {
        &self.digits
    }

    fn render_digits<'s>(&self, area: Rect, target: &mut impl Renderer<'s>) {
        let left = area.left_center() + Offset::y(Font::MONO.visible_text_height("1") / 2);
        let digits = self.digits.len();

        if digits <= MAX_VISIBLE_DIGITS {
            shape::Text::new(left, &self.digits)
                .with_align(Alignment::Start)
                .with_font(Font::MONO)
                .with_fg(self.style.text_color)
                .render(target);
        } else {
            let offset: usize = digits.saturating_sub(MAX_VISIBLE_DIGITS);
            shape::Text::new(left, &self.digits[offset..])
                .with_align(Alignment::Start)
                .with_font(Font::MONO)
                .with_fg(self.style.text_color)
                .render(target);
        }
    }

    fn render_dots<'s>(&self, area: Rect, target: &mut impl Renderer<'s>) {
        let mut cursor = area.left_center();

        let digits = self.digits.len();
        let dots_visible = digits.min(MAX_VISIBLE_DOTS);
        let step = Self::DOT + Self::PADDING;

        // Jiggle when overflowed.
        if digits > MAX_VISIBLE_DOTS + 1 && (digits + 1) % 2 == 0 {
            cursor.x += Self::TWITCH
        }

        let mut digit_idx = 0;
        // Small leftmost dot.
        if digits > MAX_VISIBLE_DOTS + 1 {
            shape::ToifImage::new(cursor, theme::DOT_SMALL.toif)
                .with_align(Alignment2D::CENTER_LEFT)
                .with_fg(theme::GREY)
                .render(target);
            cursor.x += step;
            digit_idx += 1;
        }

        // Greyed out dot.
        if digits > MAX_VISIBLE_DOTS {
            shape::ToifImage::new(cursor, theme::DOT_SMALL.toif)
                .with_align(Alignment2D::CENTER_LEFT)
                .with_fg(self.style.text_color)
                .render(target);
            cursor.x += step;
            digit_idx += 1;
        }

        // Draw a dot for each PIN digit.
        for _ in digit_idx..dots_visible {
            shape::ToifImage::new(cursor, theme::ICON_PIN_BULLET.toif)
                .with_align(Alignment2D::CENTER_LEFT)
                .with_fg(self.style.text_color)
                .render(target);
            cursor.x += step;
        }
    }
}

impl Component for PinDots {
    type Msg = Never;

    fn place(&mut self, bounds: Rect) -> Rect {
        self.pad.place(bounds);
        self.area = bounds;
        self.area
    }

    fn event(&mut self, ctx: &mut EventCtx, event: Event) -> Option<Self::Msg> {
        match event {
            Event::Touch(TouchEvent::TouchStart(pos)) => {
                if self.area.contains(pos) {
                    self.display_digits = true;
                    self.pad.clear();
                    ctx.request_paint();
                };
                None
            }
            Event::Touch(TouchEvent::TouchEnd(_)) => {
                if mem::replace(&mut self.display_digits, false) {
                    self.pad.clear();
                    ctx.request_paint();
                };
                None
            }
            _ => None,
        }
    }

    fn paint(&mut self) {
        // TODO: remove when ui-t3t1 done
    }

    fn render<'s>(&'s self, target: &mut impl Renderer<'s>) {
        let dot_area = self.area.inset(HEADER_PADDING);
        self.pad.render(target);
        if self.display_digits {
            self.render_digits(dot_area, target)
        } else {
            self.render_dots(dot_area, target)
        }
    }
}

#[cfg(feature = "ui_debug")]
impl crate::trace::Trace for PinKeyboard<'_> {
    fn trace(&self, t: &mut dyn crate::trace::Tracer) {
        t.component("PinKeyboard");
        // So that debuglink knows the locations of the buttons
        let mut digits_order = ShortString::new();
        for btn in self.digit_btns.iter() {
            let btn_content = btn.inner().content();
            if let ButtonContent::Text(text) = btn_content {
                text.map(|text| {
                    unwrap!(digits_order.push_str(text));
                });
            }
        }
        t.string("digits_order", digits_order.as_str().into());
        t.string("pin", self.textbox.inner().pin().into());
        t.bool("display_digits", self.textbox.inner().display_digits);
    }
}
