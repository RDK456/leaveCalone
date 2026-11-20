# APP SHEET — Achromatic ledger
seed 1915484187 · drawn 2026-11-20 · kind devtools · light
Brief: Lighten: Windows disk cleaner that frees C: safely, moves big app folders to D: with junctions, verifies every step

This is the style for this product. Adopt it whole and keep its move intact; everything the sheet does not fix comes from the
shared defaults in references/app-screens.md. Redraw only for a reason you can state (it contradicts the brand, or the product is
used in a way the style does not suit) and say why in your report. The values were measured on real product screens: use them
as given rather than rounding them towards your habits.

## 1. The style
The chrome holds no colour at all; money and counts are set in ink, the one chart hue marks the screen's main data element, and the only other hue is a small signal where a figure changed.

**The move:** No brand colour anywhere in the chrome: the one primary action is near-black (a grey pill, #e5e5e5, is the secondary), the main data element (today's series, the booked bars, the current period) takes the one chart hue, #1d5fbf, and everything else is drawn in ink with the comparison or the part after the cursor in grey, zero balances are set in grey, and the only hue on the screen is a small triangle or a short coloured number where a figure changed.

**Colour budget:** under 1% coloured pixels; the chrome is ink, the one chart hue goes on the main data element, and the only other hue is a small change signal.
**Off-style here:** a brand-coloured primary button; tinted cards or tinted KPI tiles; a coloured chart where one series in ink says it; bold on anything but money; a chart with no y-scale (the study's one example of this look hid magnitude behind hover and was flagged); a grey pill as the only button for the screen's main job: in testing an Approve repeated in grey on every row left the screen without a primary.
Distilled from 4 screens: a seller home, a per-agent insights page, a generated report, a settings page with usage numbers. The studied screens of this kind used 11-12px card radii, which §9.4 counts among the rare middle values; the library sets the tight pair (8 and 6) so the style lands on the bimodal rule. After testing, the chart hue moved from an unused third series colour onto the main data element: a screen whose only hue was a delta read as a kit.

**Its face.** Choose one of these (or one of your own of the same weight) before you build, write it in your notes next to
the screen's question, and check that it shows on the first screen. Without one the screen reads as a well-applied kit.
- A ground from the product's world in place of #f8f8f8, one level off white and low in chroma: a stone (#f4f3f1) for a bakery or a shop, a sage (#f2f5f2) for a garden or a greenhouse, a blue-grey (#f2f4f7) for a ledger or a meter. Cards stay white; a warm tint stays under the cream check (saturation under about 15%).
- The one chart hue (#1d5fbf, or the brand's own hue at 3:1 or more on white) on the screen's main data element: today's line, the booked bars, the current period's column, the margin that breaks. The chrome stays ink and the hue stays under 1% of the screen.
- Every figure in Geist Mono, as the style's decision, so money and counts read as a ledger's columns while the text stays Geist. Lint and the inspector flag mono; give the reason in the report.

## 2. Tokens
```css
:root {
  --ground: #f8f8f8;
  --surface: #ffffff;
  --raised: #eeeeee;
  --border: #e6e6e6;
  --rule: #f0f0f0;
  --control: #d4d4d4;
  --text: #0a0a0a;
  --muted: #6b6b6b;
  --accent: #141414;
  --positive: #2e7d32;
  --negative: #c4242a;
  --warning: #92400e;
  --chart-1: #141414;
  --chart-2: #8a8a8a;
  --chart-3: #1d5fbf;
  --pill-positive-bg: #dcfae5; --pill-positive-ink: #04250e;
  --pill-negative-bg: #ffedea; --pill-negative-ink: #831c1e;
  --pill-warning-bg: #fffbeb; --pill-warning-ink: #92400e;
  --pill-neutral-bg: #eeeeee; --pill-neutral-ink: #3d3d3d;
  --radius-card: 8px;
  --radius-control: 6px;
  --row-height: 44px;
  --sidebar-width: 256px;
  --gap: 12px;
  --pad: 24px;
  --control-height: 36px;
  --fs-title: 24px;
  --fs-section: 16px;
  --fs-kpi: 30px;
  --fs-body: 14px;
  --fs-small: 12px;
}
```
Density: sparse. One gap value, horizontally and vertically. No shadow on anything at rest.

## 3. Type — Geist
```html
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;700&family=Geist+Mono:wght@400;500&display=swap" rel="stylesheet">
```
`font-family: "Geist", Inter, system-ui, sans-serif`. Weights 400, 500, 700. Sizes: title 24px · section 16px · kpi 30px · body 14px · small 12px. 700 is for money figures only; every other number is 400-500. Second face: Geist Mono, for IDs and keys only.
Numbers: `font-variant-numeric: tabular-nums` on every table, KPI and anything that updates. Sentence case; no tracked capitals.

## 4. Shell — Icon rail
An unlabelled rail of 50-60px carries the brand mark at the top and 15-18px outline icons on a 34-49px pitch; groups are split by 20px hairlines, utilities sit at the bottom. The rail sits on the page ground with a 1px edge, or is white with a faint 4px shadow on a grey ground. It saves about 180px of width against a labelled sidebar, so it suits products with few destinations and screens that need the width. Icons need tooltips and an accessible name.
Numbers: railWidth 50-60, median 55; icon 15-18px outline; pitch 34-49; active icon colour alone, or a full-bleed 50px band at a 2% tint, or a 1px accent bar on the rail's border; groups short 20px hairlines, no labels; utilities pushed to the bottom after a gap of about 200px.
On a phone: The rail becomes a bottom bar with the same four or five icons, each with a short label under it.

## 5. Screens

### overview — Platform overview with a usage tray
Sections stacked in one column about 48px apart, each with an 18/500 title on the left and a quiet action on the right ('View all', an outlined Refresh, a date-range button), and freshness in 14px grey after the title with a small status dot. First, a getting-started strip: one container split by 1px dividers into 4 equal cells (16/500 title, a 14px description, a grey arrow top right), shown until setup is done. Then the recent items table, capped at 4-5 rows on a 49px pitch: IDs in mono truncated to 8 characters plus an ellipsis with a copy icon, relative expiry in grey, rules that bleed 8px past the text column. Then usage: a #fafafa tray with 6px padding holding a KPI strip of 5 equal cells (a 14/500 label over a 24/400 value with a 14px unit suffix, '5 mins') and, 6px below, a chart card whose metric is chosen by the KPI cells themselves (the selected cell marked by a 2px rule or the active fill), never a second control repeating the KPI labels; the chart takes two-thirds of the width when its line is flat for most of the window, with the next answer beside it. Durations and timestamps in tabular figures. A missing value is a grey dash at label size, not '--' at value size.
On a phone: The getting-started strip becomes a 2 x 2 grid or a list; the table becomes two-line rows; the KPI strip a 2-column grid.

### list — Table with tabs and a filter bar
Title row: an 18-24px title on the left, at most an outlined secondary and the one primary (36px) on the right. Underline tabs for the list's states on a full-width 1px rule with a 2px indicator as wide as the label; 22-32px below, the toolbar: a search of about 252-316 x 32-36 on the left, the count ('7 events') and the sort on the right. The table sits on the panel or page ground with no card around it: a header of 12-13px grey sentence-case labels with no fill and no rule, rows on a 48-56px pitch for one line or 72-92px with a thumbnail (media height plus 24-32px), 1px rules 3-7% darker than the ground or no rules at all, the identity column given the most width, numbers right-aligned in tabular figures with the unit muted ('5 /10'), one status mark per row and one kebab at the far right. Drop a column that is empty in every row (a blank co-host column left a 300px dead band). On an overview, cap the rows and link to the full list; on the full list, state the total and load more at the end.
On a phone: Rows become two-line items: identity and the key number on line 1, the secondary fields in 13px grey on line 2, the kebab at the right. Tabs scroll in their own row; the search becomes a full-width field.

### detail
No pattern in the library for this screen. Compose it from the rules in references/app-screens.md.

### settings — Settings in two columns
A centred column of about 1152px. Each section is one row: a label column of about 450px (16/500 title, 14px grey description; the title's cap top 6-8px below the panel top, level with the panel's first line) beside a panel of about 700px with a #fafafa fill, 16px radius, 16px padding, no border and no shadow. Sections are split by a full-width 1px rule with 32px above and below. Inside a panel: field labels 14/500 above 40px white inputs (1px #e8e8e8, radius 10), 8px from label to field, 16px between fields, 24px from the description to the first label, 16px to the Save button; switch rows with a 14/500 title, a 20px status badge 8px after it, a 14px grey description and a 28x16 toggle 16px from the right edge, centred on the title line. A plan-gated feature shows a 48px tinted strip with a 32px button directly above its panel, and the panel stays readable. When a panel runs much taller than its label, put the section's help or a summary of the current values in the label column rather than 250px of nothing.
On a phone: Each section stacks: title and description, then the panel at full width. Inside, label, 8-10px, field, 16-26px to the next pair; toggles stay on the right of their title.

## 6. Always
Real data for this product, written before the markup: names, amounts that add up, dates in order, units on every figure,
honest zeros and dashes. One chart answers one question. No gradients, glows, glass, icons in tinted squares, sparklines on KPIs,
rainbow charts, emoji, lorem or Acme. `<skill>/assets/app.css` and `<skill>/assets/charts.js` are a neutral start set to these
rules. Check with `node <skill>/scripts/lint.mjs <files>` and `node <skill>/scripts/inspect.mjs <page>`: this sheet switches both
to app rules.

Saved to F:/projects/speed/lighten/.taste-skill/direction.json · reprint with `node draw.mjs --show`
