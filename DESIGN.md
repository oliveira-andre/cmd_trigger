## Overview

CmdTrigger is a command and automation tool built for developers, and the design wears that audience proudly. A near-black `{colors.canvas}` (`#101010`) page background runs edge-to-edge, with no light-mode counterpart. A single electric-green accent (`{colors.primary}` `#00d992`) is reserved for CTAs, status pills and the `>_` prompt mark. The typography pairs sentence-case Inter with SF Mono for inline code and command snippets. The whole product reads like polished documentation that decided to also run your computer.

The decorative system is restrained: no gradient mesh, no atmospheric backdrop, no illustration suite. Instead, small typographic moments mark the identity:
- a green code chip (`cmdtrigger run …`);
- a 3-px outlined feature card sitting against the same near-black canvas;
- a green hairline divider between section bands.

The result feels engineered. Every card has a hairline border, every snippet has a copy-to-clipboard button, and every metric renders in a numeric monospace.

Type stays calm:
- **Display:** 60 px in regular weight with `-0.65 px` tracking. Not a billboard headline; more like a documentation H1.
- **Section headings:** 36 px / 24 px, in similar weights.
- **Body:** 16 px Inter at line-height 1.65, for the legibility long-form developers expect.
- **Eyebrows:** uppercase labels above section headlines, in the `EVERYTHING YOU NEED` style. They use Inter at weight 600 with wide positive tracking (`2.52 px` at 14 px).

**Key Characteristics:**
- **One accent.** Electric green `{colors.primary}` (`#00d992`) carries every CTA, every status pill and the `>_` mark. There is no second accent.
- **One surface.** The dark canvas (`{colors.canvas}` `#101010`) is the only page surface. There is no light-mode rhythm; everything is one continuous dark surface broken by feature-card boundaries.
- **Hairline cards.** Hairline-bordered feature cards (`{colors.hairline}` `#3d3a39`, 1 px solid) are the primary chrome: no shadows, no fills, just precise hairline rectangles.
- **Dashed divider.** A dashed border (`1px dashed rgba(79, 93, 117, 0.4)`) appears between sections as a quiet rhythm cue. It's the only ornamental line.
- **Two faces.** Inter and SF Mono carry every typographic role. SF Mono is reserved for code blocks, inline command snippets and metric counters.
- **Tight corners.** Buttons are 6 px rounded rectangles, not pills; only inline status tags use the 9999 px full pill.

## Colors

### Brand & Accent
- **Electric Green** (`{colors.primary}`, `#00d992`): the single accent. It carries every primary CTA, every status pill, every "live" indicator, and the `>_` mark itself. Reserved.
- **Primary Soft** (`{colors.primary-soft}`, `#2fd6a1`): a slightly more muted green, used inside ghost buttons and for tooltip and focus indicators.
- **Primary Deep** (`{colors.primary-deep}`, `#10b981`): a darker green, used for inline links in body copy and as the pressed state of primary buttons.

### Surface
- **Canvas** (`{colors.canvas}`, `#101010`): the default near-black background, and the only surface mode.
- **Canvas Soft** (`{colors.canvas-soft}`, `#1a1a1a`): a slightly lighter fill for code blocks and form inputs, so they stand apart from the canvas.
- **Hairline** (`{colors.hairline}`, `#3d3a39`): 1 px solid borders on feature cards, buttons and row dividers. The universal "edge" colour.
- **Hairline Soft** (`{colors.hairline-soft}`, `#b8b3b0`): a lighter divider tint for rare on-light secondary contexts.

### Text
- **Ink** (`{colors.ink}`, `#f2f2f2`): default text on the dark canvas, slightly off-white to reduce contrast strain.
- **Ink Strong** (`{colors.ink-strong}`, `#ffffff`): pure white for headlines and high-emphasis copy.
- **Body** (`{colors.body}`, `#bdbdbd`): secondary text, such as supporting copy and long-form paragraphs.
- **Mute** (`{colors.mute}`, `#8b949e`): lowest-priority text, such as captions, fine print and secondary footer lines.
- **Canvas Text Soft** (`{colors.canvas-text-soft}`, `#f5f6f7`): text inside code mockups, kept slightly cooler than the surrounding body text.
- **On Primary** (`{colors.on-primary}`, `#101010`): near-black text on green buttons.

### Semantic
There is no separate error / warning palette for decoration. Success cues use the primary green, and missing or idle states use muted grey.

The product does need to say when something failed (a run exited with an error). For that one job it uses a functional **Danger** red (`{colors.danger}`, `#f6465d`), as status text or a tinted tag, never as a fill or an accent. Waiting states (queued, running) use **Primary Soft**.

## Typography

### Font Family
Two faces carry the system:
1. **Inter** for every display, body, button and link role. Weights 400 / 500 / 600 / 700 are the working set. Use it with the OpenType features `"calt"` and `"rlig"`, so Inter's contextual alternates render correctly.
2. **SF Mono** for inline code, command snippets, terminal mockups and numeric counters, with `SFMono-Regular` and Menlo / Monaco / Consolas / Liberation Mono as fallbacks. Weights 400 / 549 / 550 / 700 are present. The unusual 549 / 550 sub-bold weight gives the mono a "slightly heavier than regular" voice for emphasis.

### Hierarchy

| Token | Size | Weight | Line Height | Letter Spacing | Use |
|---|---|---|---|---|---|
| `{typography.display-xl}` | 60px | 400 | 60px | -0.65px | Hero headline. |
| `{typography.display-lg}` | 36px | 400 | 40px | -0.9px | Section headlines. |
| `{typography.display-md}` | 24px | 700 | 32px | -0.6px | Sub-section / card-title displays, page titles. |
| `{typography.display-sm}` | 20px | 600 | 28px | 0 | Card titles in dense grids. |
| `{typography.eyebrow-mono}` | 14px | 600 | 20px | 2.52px | UPPERCASE eyebrow tags ("EVERYTHING YOU NEED"). |
| `{typography.eyebrow-uppercase}` | 18px | 600 | 28px | 0.45px | Larger uppercase eyebrows above hero subsections. |
| `{typography.body-lg}` | 18px | 400 | 28px | 0 | Lead paragraphs. |
| `{typography.body-md}` | 16px | 400 | 26px | 0 | Default body paragraph. |
| `{typography.body-md-strong}` | 16px | 600 | 24px | 0 | Bolded inline body. |
| `{typography.body-sm}` | 14px | 400 | 20px | 0 | Secondary body; the default in dense app UI. |
| `{typography.body-sm-strong}` | 14px | 600 | 23px | 0 | Bold caption / pill-tag labels. |
| `{typography.caption}` | 12px | 400 | 16px | 0 | Fine print. |
| `{typography.caption-strong}` | 12px | 500 | 16px | 0 | Bold caption. |
| `{typography.code}` | 13px | 400 | 18px | 0 | Code blocks, inline command snippets. |
| `{typography.code-strong}` | 13px | 550 | 16px | 0 | Emphasised inline code (SF Mono's "almost-bold" weight). |
| `{typography.button-md}` | 16px | 600 | 24px | 0 | Button labels. |

### Principles
- **Calm display.** Inter regular at 60 px is the counter to the tendency of developer-tool marketing to shout. The light tracking and modest weight read like documentation.
- **Two-face contrast carries the technical voice.** Inter for narrative; SF Mono for anything that could be typed at a terminal.
- **The tracked uppercase eyebrow is the signature label.** `2.52 px` at 14 px is the documented value.

### Note on Font Substitutes
- **Sans:** Inter, self-hosted (`@fontsource/inter`).
- **Mono:** SF Mono is an Apple system font. JetBrains Mono (bundled, `@fontsource/jetbrains-mono`) or Geist Mono are the best free substitutes off Apple platforms.

## Layout

### Spacing System
- **Base unit**: 4 px; small 5 / 6.4 px values appear inside code-mockup line-height compensation.
- **Tokens**:
  - `{spacing.xxs}` 2 px · `{spacing.xs}` 4 px · `{spacing.sm}` 8 px · `{spacing.md}` 12 px
  - `{spacing.lg}` 16 px · `{spacing.xl}` 20 px · `{spacing.2xl}` 24 px · `{spacing.3xl}` 32 px
  - `{spacing.4xl}` 40 px · `{spacing.5xl}` 48 px · `{spacing.6xl}` 64 px
- **Section padding**: hero and content bands use `{spacing.5xl}` 48 px top and bottom.
- **Card interior padding**: feature cards sit at `{spacing.2xl}` 24 px.

### Grid & Container
- **Container:** the marketing container centres at roughly 1200–1400 px. Colour runs edge to edge, with horizontal gutters of `{spacing.3xl}` on desktop.
- **Feature-card grids:** 2-up to 3-up on desktop, 1-up on mobile.

### Responsive Strategy

#### Breakpoints

| Name | Width | Key Changes |
|---|---|---|
| Mobile | < 768px | Hero 60→32 px; cards 1-up; nav hamburger. |
| Tablet | 768–1023px | Cards 2-up; nav stays horizontal. |
| Desktop | ≥ 1024px | Full 3-up card grids. |

#### Touch Targets
Buttons render about 44 px tall (12 px vertical padding plus a 24 px line height), which meets WCAG AAA at every breakpoint.

#### Collapsing Strategy
- **Nav:** collapses to a hamburger on mobile. The menu overlay keeps the same green CTA pinned at the bottom.
- **Grids:** feature-card grids drop to 1-up.
- **Hero:** the typography scales fluidly.

#### Image Behavior
Code-editor mockups render as image-like cards with copy-to-clipboard affordances. No photography.

## Elevation & Depth

| Level | Treatment | Use |
|---|---|---|
| Level 0 — Flat | No shadow, no border. | Full-bleed bands. |
| Level 1 — Hairline | 1 px solid `{colors.hairline}` border on `{colors.canvas}`. | Default for every feature card and button. |
| Level 2 — Inset Glow | `0 0 15px rgba(92, 88, 85, 0.2)` subtle outer glow. | Hovering / featured cards. |
| Level 3 — Modal Stack | `0 20px 60px rgba(0,0,0,0.7), 0 0 0 1px rgba(148,163,184,0.1) inset` heavy drop + inset ring. | Modal / dialog surfaces. |

### Decorative Depth
- **Hairline cards on dark canvas:** the only true elevation mode.
- **Featured or active:** a 2 px solid `{colors.primary}` green border occasionally marks a card as featured or active.
- **Section rhythm:** a 1 px dashed `rgba(79, 93, 117, 0.4)` divider sits between section bands as a quiet cue.

## Shapes

### Border Radius Scale

| Token | Value | Use |
|---|---|---|
| `{rounded.none}` | 0px | Full-bleed bands. |
| `{rounded.xs}` | 4px | Smallest inline pills, code inline chips. |
| `{rounded.sm}` | 6px | Default button and input radius. |
| `{rounded.md}` | 8px | Card chrome, code-block chrome. |
| `{rounded.pill}` | 9999px | Inline status tags ("Live", "Beta"). |
| `{rounded.full}` | 9999px | Circular icon containers. |

## Components

### Buttons

**`button-primary`**: the electric-green CTA.
- Background `{colors.primary}`, text `{colors.on-primary}` (near-black), label `{typography.button-md}`.
- Padding `{spacing.md} {spacing.lg}`, shape `{rounded.sm}` 6 px.

**`button-outline-on-dark`**: the hairline-on-dark secondary button.
- Background `{colors.canvas}`, text `{colors.ink}`, 1 px solid `{colors.hairline}` border.
- Same typography, padding and shape as the primary button.

**`button-ghost-green`**: text-only with a green label, for tertiary actions.
- Background `{colors.canvas}`, text `{colors.primary-soft}`, no border.

**`button-pill-tag`**: the inline pill for category tags and status labels.
- Background `{colors.canvas}`, text `{colors.ink}`, hairline border, body in `{typography.body-sm}`.
- Padding `{spacing.xs} {spacing.md}`, shape `{rounded.pill}` 9999 px.

### Cards & Containers

**`card-feature`**: the default feature card, and the most-repeated card chrome.
- Background `{colors.canvas}`, text `{colors.ink}`, 1 px solid `{colors.hairline}` border.
- Padding `{spacing.2xl}`, shape `{rounded.md}` 8 px.

**`card-feature-emphasized`**: the same chrome as `card-feature`, with a 3 px solid `{colors.hairline}` border for emphasis.

**`code-mockup`**: the dark code-editor card with a copy-to-clipboard affordance.
- Background `{colors.canvas}`, text `{colors.ink}`, 1 px solid `{colors.hairline}`.
- Body in `{typography.code}` (SF Mono 13 px), padding `{spacing.xl}`, shape `{rounded.md}`.

**`code-inline-chip`**: the inline command snippet pill.
- Background `{colors.canvas-soft}`, text `{colors.canvas-text-soft}`, body in `{typography.code}`.
- Padding `{spacing.xxs} {spacing.sm}`, shape `{rounded.sm}`.

### Inputs & Forms

**`text-input`**: the standard text input on dark.
- Background `{colors.canvas-soft}`, text `{colors.ink}`, 1 px solid `{colors.hairline}`, body in `{typography.body-sm}`.
- Padding `{spacing.md} {spacing.lg}`, shape `{rounded.sm}` 6 px.

### Navigation

**`nav-bar`**: the sticky top nav on dark. Background `{colors.canvas}`, text `{colors.ink}`, padding `{spacing.md} {spacing.3xl}`.

**`nav-link`**: link items in the nav. Text `{colors.body}`, set in `{typography.body-sm}`.

**`footer`**: the dark footer band. Background `{colors.canvas}`, text `{colors.body}` in `{typography.body-sm}`, padding `{spacing.4xl} {spacing.3xl}`.

### Signature Components

**`hero-band`**: the dark hero band with the 60 px Inter headline.
- Background `{colors.canvas}`, text `{colors.ink}`, padding `{spacing.5xl} {spacing.3xl}`.
- Headline in `{typography.display-xl}` (60 px, weight 400, `-0.65 px` tracking), in `{colors.ink-strong}` white.
- Above it, an eyebrow in `{typography.eyebrow-mono}` (uppercase, tracked).

**`content-band`**: the standard content band hosting feature grids.
- Background `{colors.canvas}`, text `{colors.ink}`, padding `{spacing.5xl} {spacing.3xl}`.
- Section headline in `{typography.display-lg}`.

**`green-divider-band`**: a thin green-glow band that occasionally separates major sections. Background `{colors.canvas}`, with a 2 px solid `{colors.primary}` border top and bottom. The only chromatic divider.

### Examples (illustrative)

Reference surfaces built from the primitives above, so new screens re-use the same chrome:

| Surface | Built from |
|---|---|
| **`ex-pricing-tier`** | Feature-card chrome on the canvas-soft surface. |
| **`ex-pricing-tier-featured`** | The featured version, marked with the 2 px green border (there is no light mode to flip to). |
| **`ex-product-selector`** | A "what's included" summary card. |
| **`ex-cart-drawer`** | A summary list, with hairline dividers between line items. |
| **`ex-app-shell-row`** | A sidebar nav row; the active row is marked with the primary green. |
| **`ex-data-table-cell`** | Table cells: the header in the eyebrow typography, the body in body-sm, hairline row borders. |
| **`ex-auth-form-card`** | Feature-card chrome holding text inputs. |
| **`ex-modal-card`** | Feature-card chrome with the Level 3 modal shadow. |
| **`ex-empty-state-card`** | A feature card with a caption-typography message. |
| **`ex-toast`** | Feature-card shape with a hairline and a soft shadow. |

## Do's and Don'ts

### Do
- **Reserve the green.** Use `{colors.primary}` (`#00d992`) for every primary CTA, the `>_` mark and live-status indicators. The green is the centre of gravity.
- **Stay on the canvas.** The dark `{colors.canvas}` (`#101010`) is the only page surface. There is no light-mode rhythm.
- **Use hairlines, not shadows.** Build cards with 1 px `{colors.hairline}` borders; hairlines on dark *are* the elevation system.
- **Pair the two faces.** Use Inter for sentence-case text and SF Mono for inline code and command snippets. Every uppercase moment is Inter at weight 600 with `2.52 px` tracking, not a separate mono.
- **Keep the radii.** Use `{rounded.sm}` 6 px for buttons, `{rounded.md}` 8 px for cards, and `{rounded.pill}` 9999 px only for inline status tags.

### Don't
- **No light mode.** The design is dark-canvas only, dialogs and sheets included.
- **No green body text.** The primary green is for CTAs, never a body-text fill.
- **No soft drop shadows on cards.** Use hairlines and the occasional glow, never material shadows.
- **No heavy display weights.** Keep the large display headline calm at weight 400, not 700+.
- **No other families.** Don't replace Inter or SF Mono; both faces are part of the voice and the pairing.

## In CmdTrigger

| Surface | Where the tokens live | Notes |
|---|---|---|
| Mac app (Tauri window) | `src/styles.css` `:root` | Dense app UI: `{typography.body-sm}` 14 px is the base size. Page titles use `{typography.display-md}`. Section labels use the tracked uppercase eyebrow. |
| Server dashboard | `crates/cmdtrigger-server/dashboard/app.css` | Phone-first, `{typography.body-md}` 16 px base. |
| iPhone app | `ios/CmdTrigger/Theme.swift` | System Inter isn't available, so SF Pro and SF Mono stand in: same sizes, weights and tracking. |

Tiles, rows and panels are `card-feature`s: canvas plus a hairline. Inputs and code use `canvas-soft`. Dialogs and sheets are dark too (`canvas-soft` with the Level 3 shadow); never light.
