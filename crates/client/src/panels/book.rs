//! Reading: the original's `ItemTextFrame` for the books, letters and plaques of the world and of the pack, a page at a time, on the material the page is on
//! (parchment, stone, marble, silver, bronze, valentine).

use super::*;
use crate::uixml::HTML_RUNS;

pub(super) const BOOK: &str = "ItemTextFrame";
/// What `PageTextMaterial.dbc` names its rows, and the colours of the text and of the title on each (`MATERIAL_TEXT_COLOR_TABLE`: the valentine one has the default).
const MATERIALS: [&str; 7] = ["Parchment", "Parchment", "Stone", "Marble", "Silver", "Bronze", "Valentine"];
const TEXT_COLORS: [[f32; 3]; 7] = [[0.18, 0.12, 0.06], [0.18, 0.12, 0.06], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], [0.12, 0.12, 0.12], [0.18, 0.12, 0.06], [0.18, 0.12, 0.06]];

/// What is being read: the title, the page on show (an entry of `classic_pages.ron`), its number in the text and the material (a row of `PageTextMaterial.dbc`; 0 is parchment).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Book {
    title: String,
    page: u32,
    number: u32,
    material: u8,
    /// The pages before this one.
    before: Vec<u32>,
}

/// The text of a page for the screen, in stretches that are aligned the same way ("LEFT", "CENTER" or "RIGHT"). A plain page is one left aligned stretch, `$B` is a line
/// break. A page of HTML breaks lines at `<BR>` and at the end of a paragraph or heading (the line breaks of its source are
/// blanks), and aligns each paragraph or heading by its `align` (left without one), as the original's `SimpleHTML` does.
fn runs(text: &str) -> Vec<(&'static str, String)> {
    let rest = text.replace("$B", "
").replace("$b", "
");
    let plain = rest;
    let html = plain.trim_start().to_ascii_uppercase().starts_with("<HTML>");
    if !html {
        return vec![("LEFT", plain)];
    }
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut align = "LEFT";
    let mut push = |align: &'static str, s: &str| match out.last_mut() {
        Some((a, text)) if *a == align => text.push_str(s),
        _ => out.push((align, s.to_string())),
    };
    // Whether a blank would begin a line or follow a blank.
    let mut blank = true;
    let mut chars = plain.chars();
    while let Some(c) = chars.next() {
        if c != '<' {
            // The blanks of the source (and the line breaks in it) are one blank, and none begins a line.
            if !c.is_whitespace() {
                push(align, c.encode_utf8(&mut [0; 4]));
                blank = false;
            } else if !blank {
                push(align, " ");
                blank = true;
            }
            continue;
        }
        // A tag is what is up to the next '>'; text such as "<The pages are covered in runes.>" has no tag name of the list and stays.
        let tag: String = chars.clone().take_while(|&c| c != '>').collect();
        let (closing, name) = (tag.starts_with('/'), tag.trim_start_matches('/').split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("").to_ascii_uppercase());
        if !matches!(name.as_str(), "HTML" | "BODY" | "P" | "BR" | "H1" | "H2" | "H3") {
            push(align, "<");
            blank = false;
            continue;
        }
        chars.nth(tag.chars().count());
        match (name.as_str(), closing) {
            ("BR", false) => (push(align, "\n"), blank = true).1,
            ("P" | "H1" | "H2" | "H3", false) => {
                let lower = tag.to_ascii_lowercase();
                align = ["center", "right"].iter().position(|a| lower.contains(&format!("align=\"{a}\""))).map_or("LEFT", |i| ["CENTER", "RIGHT"][i]);
            }
            ("P" | "H1" | "H2" | "H3", true) => {
                push(align, "\n");
                (align, blank) = ("LEFT", true);
            }
            _ => {}
        }
    }
    out.retain(|(_, t)| !t.is_empty());
    out
}

impl Panels {
    /// Open the window on page `page` (the first of a text) of something called `title`, on `material`. Nothing opens for a page the game does not have.
    pub fn open_book(&mut self, title: &str, page: u32, material: u8) {
        if content::page(page).is_none() {
            return;
        }
        // A book read from the pack leaves the pack open.
        let pack = self.backpack_open();
        self.close_all();
        self.tree.show(BACKPACK, pack);
        self.book = Some(Book { title: title.into(), page, number: 1, material, before: Vec::new() });
        self.tree.show(BOOK, true);
        self.events.push(Sfx::Named("igMainMenuOpen"));
        self.fill_book();
    }

    pub fn book_open(&self) -> bool {
        self.book.is_some()
    }

    pub fn close_book(&mut self) {
        if self.book.take().is_some() {
            self.events.push(Sfx::Named("igMainMenuClose"));
        }
        self.tree.show(BOOK, false);
        self.after_close();
    }

    /// The page after this one, or the one before it.
    pub(super) fn turn_book(&mut self, forward: bool) {
        let Some(book) = &mut self.book else { return };
        if forward {
            let Some(next) = content::page(book.page).map(|p| p.next).filter(|&n| n != 0) else { return };
            book.before.push(std::mem::replace(&mut book.page, next));
        } else {
            let Some(before) = book.before.pop() else { return };
            book.page = before;
        }
        book.number = book.before.len() as u32 + 1;
        self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
        self.fill_book();
    }

    /// `ItemTextFrame_OnEvent` for ITEM_TEXT_READY: the title, the text in the colour of the material, the pictures of the material and the buttons of the pages.
    fn fill_book(&mut self) {
        let Some(book) = self.book.clone() else { return };
        let Some(page) = content::page(book.page) else { return };
        let material = (book.material as usize).min(MATERIALS.len() - 1);
        let [r, g, b] = TEXT_COLORS[material];
        self.tree.set_text("ItemTextTitleText", &book.title);
        // The text begins and ends with a line, as the original has it; the stretches beyond the last region join it.
        let mut stretches = runs(&self.speak(&page.text));
        while stretches.len() > HTML_RUNS {
            let last = stretches.pop().expect("more than none").1;
            stretches.last_mut().expect("one left").1.push_str(&last);
        }
        if let Some(first) = stretches.first_mut() {
            first.1.insert(0, '\n');
        }
        if let Some(last) = stretches.last_mut() {
            last.1.push('\n');
        }
        for i in 0..HTML_RUNS {
            let region = format!("ItemTextPageTextRun{i}");
            let (align, text) = stretches.get(i).map_or(("LEFT", ""), |(a, t)| (a, t.as_str()));
            self.tree.set_text(&region, text);
            self.tree.set_region_justify(&region, align);
            self.tree.set_text_color(&region, [r, g, b, 1.0]);
        }
        for part in ["TopLeft", "TopRight", "BotLeft", "BotRight"] {
            let region = format!("ItemTextMaterial{part}");
            let parchment = material <= 1;
            self.tree.set_region_hidden(&region, parchment);
            if !parchment {
                self.tree.set_texture(&region, Some(&format!(r"Interface\ItemTextFrame\ItemText-{}-{part}", MATERIALS[material])));
            }
        }
        let (before, next) = (!book.before.is_empty(), page.next != 0);
        self.tree.set_text("ItemTextCurrentPage", &book.number.to_string());
        self.tree.set_region_hidden("ItemTextCurrentPage", !(before || next));
        self.tree.show("ItemTextPrevPageButton", before);
        self.tree.show("ItemTextNextPageButton", next);
        self.tree.reset_scroll("ItemTextScrollFrame");
    }
}

/// The stretches of text are one under the other, each as high as its text (known from the last measure).
pub(super) fn place_runs(tree: &mut Tree) {
    let mut y = 0.0;
    for i in 0..HTML_RUNS {
        let region = format!("ItemTextPageTextRun{i}");
        tree.set_region_point(&region, Point::TopLeft, Some("ItemTextPageText"), Some(Point::TopLeft), 0.0, -y);
        y += tree.text_height(&region);
    }
}

pub(super) fn setup(tree: &mut Tree) -> Result<()> {
    tree.instantiate(BOOK)?;
    // The pictures of the material show where the page is not parchment.
    tree.show("ItemTextStatusBar", false);
    tree.show(BOOK, false);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::runs;

    #[test]
    fn a_page_is_shown_in_stretches_by_their_alignment() {
        assert_eq!(runs("One$BTwo$b$bThree"), [("LEFT", "One
Two

Three".to_string())]);
        assert_eq!(runs("<The pages are covered in ancient elven runes.>"), [("LEFT", "<The pages are covered in ancient elven runes.>".to_string())]);
        // The source's own line breaks are blanks; the headings and paragraphs end lines, and each aligns itself.
        let page = "<HTML>
<BODY>
<BR/>
<H1 align=\"center\">
Eliza
</H1>
<BR/>
<P align=\"center\">
Beloved wife,
may the earth sate you.
</P>
<P align=\"right\">Yours</P>
<P>Me</P>
</BODY>
</HTML>";
        let stretches: Vec<(&str, String)> = runs(page);
        let shown: Vec<(&str, &str)> = stretches.iter().map(|(a, t)| (*a, t.as_str())).collect();
        assert_eq!(shown, [("LEFT", "\n"), ("CENTER", "Eliza \n"), ("LEFT", "\n"), ("CENTER", "Beloved wife, may the earth sate you. \n"), ("RIGHT", "Yours\n"), ("LEFT", "Me\n")]);
    }
}
