# MCHOSE Native design reference

The explicit design direction is the existing M HUB device configuration interface, recreated with native egui widgets and GPU drawing. The original is the requested reference, so its device illustrations, restrained blue selection color, profile rail, horizontal tabs, compact controls and rounded setting surfaces are intentional. The native application must not embed a browser, HTML, a WebView or JavaScript runtime.

## Evidence and limits

Source snapshot: https://www.mchose.com.cn/#/newMouse?deviceName=MCHOSE+A7+V3+Ultra%2B , downloaded 2026-10-22. Public compiled Vue source is archived under `research/upstream`. Source identifiers below refer to `purify.es-BufdR6vO.js` and `purify-_I2KCF9a.css`; design tokens come from `index-CY28rP7W.css`. `research/browser/purify.pretty.js` is a formatting-only research derivative for readable line references.

The isolated Chromium screenshot `research/browser/original-start.png` verifies the top bar and background. Without authorizing WebHID the deep link cannot initialize the device and stays on a spinner. Connected page geometry is therefore source-derived, not a visually verified connected screenshot. The original user browser and WebHID permissions were untouched.

## Palette

| Role / upstream token | Light | Dark |
|---|---|---|
| Outer background `w29` | #E6E7ED | #0C0C0C |
| Main panel `w30` | #FFFFFF | #141414 |
| Settings surface `w12` | #F2F3F7 | #1A1A1B |
| Primary text `w1` | #151515 | #FFFFFF |
| Secondary text `w2` | primary 50% opacity | primary 50% opacity |
| Border `w3` | #D7DEE3 | #2B2B2B |
| Control background `w17` | #FFFFFF | #303134 |
| Accent `w37` | #0053E2 | #226BEE |
| Selected tint `b7` | accent 10% | accent 10% |
| Success `n19` | #32AA70 | #32AA70 |
| Warning `n23` | #F59C25 | #F59C25 |
| Error `n10` | #EA5E56 | #EA5E56 |

DPI stage colors are semantic identifiers: red #EA5E56, blue accent, green #32AA70, yellow #F5E025, cyan #00DDE4, purple #B500EC. They should not spread into unrelated decorative elements.

## Geometry and type

* Global top bar: 60 px; source screenshot is 1440 × 1000.
* Main content: profile rail at left, flexible panel at right. Panel has 12 px corner radius and 24 px right/bottom outside margin. Section tabs are horizontal in a 68 px high row, with 24 px horizontal padding.
* Key assignment: palette/search panel is 336 px wide, 12 px radius, margin 24 px; mouse area padding 50 × 75 px; mouse render is 252 × 476 px (native asset is exactly twice this). Mapping labels use connector lines and anchor dots tied to physical mouse buttons.
* Standard performance panels: 186 px high, 24 px padding, 12 px radius. Performance-mode and debounce panel minimum height 136 px. Gaps are 16 px. Heading 14 px semibold, help copy 13 px regular.
* DPI stage count segmented buttons: 58/59 px width, 32 px line height, 8 px outer corners. Slider rails are 8 px, thumbs 16 px, labels 12 px.
* General buttons: 40 px high, 8 px radius, 14 px normal. The original uses MiSans with Helvetica/system fallbacks. Native text uses a Cyrillic-capable sans font for readable Russian, not a monospace aesthetic.
* Keep shadows light and sparse: original main panel uses 0 4 px 12 px rgba(0,0,0,.08). Native rendering can omit the shadow when contrast already separates the surface.

## Native adaptation

Use the same visual hierarchy with light/dark switch. Keep the persistent mouse name, connection/battery state and onboard profile selection separate from tab contents. Dock is a visible Lighting tab associated with MagDock. Expose macros and local import/export directly so they are discoverable without the original drag/drop-only flows. A disconnected device shows an explicit state and refresh action; unknown battery and firmware are blank/unknown rather than fake defaults. Staged controls indicate unsaved changes and commit on an explicit apply action.

Target window 1440 × 940, minimum 1000 × 700. Scroll long settings instead of clipping them. Under 1180 px reduce the profile rail; below a useful two-column width use one column. Keyboard focus and Tab navigation must work for every control. The only animations are interaction feedback or progress from actual operations. Do not animate battery/status dots.

Design dials: density 7/10 (configuration tool); color 3/10 (mostly neutral with functional color); motion 1/10 (interaction only); ornament 1/10 (actual product render only); rhythm 6/10 (device diagram, DPI sliders, forms and macro table each fit their data).

Delivery-gate scale: ENERGY 1 / RHYTHM 2 / MOTION 1. This is a compact device utility: the product diagram and current editor are the focal points, and animation is limited to interaction feedback.

## Assets

Official public renders were taken from URLs embedded in the distributed Hub. `assets/provenance.json` records full source URLs, SHA-256, bytes and date. Mouse renders are transparent 504 × 952 PNGs in white, black, blue, red and pink; MagDock is transparent 614 × 890. These are source assets, not generated substitutes. No third-party branding or endorsement claims are added.

## Native accessibility adjustments

The source's half-opacity secondary text is raised to #9E9E9E in dark mode and #64676F in light mode. Native slider rails use #767B87 on #1A1A1B (4.10:1) and #747B8A on #F2F3F7 (3.83:1), with a 6 px rail and circular thumb. Selected controls always use white text: 4.77:1 on dark-theme blue and 6.30:1 on light-theme blue. Small accent text uses #75A7FF in dark mode while filled controls retain #226BEE. Light-theme error text and the recording stop button use #B42323; the stop button always has white text. The device rail scrolls as one region at short window heights, keeping the color selector below MagDock instead of overlapping it.

## Ace75 keyboard reference (2026-10-27)

The Ace75 editor follows the keyboard interface at `https://www.mchose.com.cn/#/keyboard`, whose device editor is the `/cizhou` Next application. Its keyboard is a vector diagram with physical key positions, rather than a photograph. The source snapshot is already archived in `research/ace75/upstream`; the source-derived descriptions below do not claim a live connected-browser screenshot.

### Source pointers

- `analysis/layout-ace75_8k.pretty.js`, module 37685: chassis SVG. Module 27409: Ace75-specific RT, wheel, logo and LED layout. The checked-in `assets/ace75-layout.json` preserves the source geometry and matrix addresses.
- `analysis/next/2233-a1ac28ebe5f04af2.js:935`: key wrapper, neutral color fallback and selected colors. `:16484` contains the RT vector and `:16559` the Ace75 wheel vector.
- `upstream/cizhou/_next/static/css/30c9ff4b04b7d072.css`: actual light/dark keyboard tokens. `ceb4ed2f418eaa65.css` defines `.key-wrapper`; `3dca930d1774b003.css` defines the layered keyboard container.
- `analysis/next/1833-bb9dc82e7ca4286d.js:17387`: pane and setting-card building blocks; `:26436`: performance card; `:27225`: three-part trigger editor; `:24947`: 241×241 switch/spring illustration; `:29635`: device rail and main content shell.

Paths above are relative to `research/ace75`. The existing REA source inventory is retained under parent evidence `ev_5c317e437ae9eb49eadd702a7835f8af258c284c5ff8a3ca4c09611852119998`. CSS and component bodies were inspected as source; an isolated pure-component extraction is recorded in `research/ace75/design-reference/spring-provenance.json`. No connected browser, keyboard writes or application bootstrap was used for this design research.

### Keyboard appearance

| Element | Light | Dark |
| --- | --- | --- |
| Keyboard chassis | `#F8F9FB` | `#303134` |
| Neutral keycaps | `#E5E9F0` | `#212225` |
| Border | `#D7DEE3` | `#2B2B2B` |
| Primary key label | `#151515` | `#FFFFFF` |
| Selected/hover outline | `#0053E2` | `#226BEE` |
| Selected fill | accent at 10% | accent at 10% |
| Setting surface | `#F2F3F7` | `#1A1A1B` |

The source chassis occupies a 793×330 coordinate system: rounded rectangle at x=11.5, y=0.5, width=781, height=329 and radius=15.5. Keys use a 42 px unit, 4 px corner radius, 1 px border and centered 12 px semibold labels (11 px in lighting previews). The RT button sits above the upper-right keys at x=682, y=-25, size=42×21. The left wheel is an 11×43.5 curved shape at x=0, y=47.45. The main keyboard contains 81 physical keys; lighting additionally includes five Space sections, the 28-section strip and the MCHOSE logo.

In the original key renderer, pure black RGB is a neutral-background sentinel: `keyColor && value(keyColor) != 0` is required before a color replaces the normal keycap. A stored black color therefore should not turn the entire light-theme keyboard into solid black keys. This is a display rule; the stored RGB values remain device data. Selection needs a visible outline and a light blue fill, rather than relying solely on a small color change. Physical key labels and any current remapping/trigger value should remain distinguishable.

The source MCHOSE logo on the lower-right bezel is a 30×24 SVG with three paths, embedded in module 27409. A standalone unchanged path extraction is available at `research/ace75/design-reference/original-ace75-logo.svg`. The app already has official logo assets under `assets/mchose-hub-logo-dark-original.png`, `assets/mchose-hub-logo-light-original.png` and `assets/mchose-logo-original.png`, with their provenance in `assets/provenance.json`. There is no need to generate a substitute brand mark.

### Settings hierarchy

The original uses a 68 px horizontal tab header with 24 px side padding. Its profiles/device rail is 320 px wide; a narrower native rail is appropriate so the keyboard and controls remain readable at 855 px. The main panel has a 12 px radius, outside right/bottom spacing of 24 px and a restrained shadow. Primary attention belongs to the keyboard, the selected key(s), and the settings that will affect them.

Original setting cards use 24 px padding, 24 px internal gaps, 12 px corners for large cards and the neutral setting surface. Performance cards have a 14 px semibold title, 13 px description, a control at the right and a minimum height of 124 px. Their source layout is two equal columns separated by 16 px. A native form should preserve this title/description/control hierarchy while allowing long Russian labels to wrap.

The trigger page has three semantic groups below the keyboard: actuation, Rapid Trigger, and advanced magnetic settings. Actuation has switch selection plus a switch/spring drawing and a vertical millimetre ruler. RT uses separate press/release controls with yellow/green semantic markers and options for separate thresholds and continuous mode. Advanced settings contain Tachyon precision and dead zones. Native layout can use two columns or stack these groups in a narrow window; controls and their units should remain visible together.

The original trigger entry selects WASD in its UI state (`iX`, source around line 27255). A one-time default selection in the native editor can make the initial form useful, provided it is only a UI selection and does not modify device settings. Later user selections and unsaved drafts must remain intact. Selected-key names/count should be stated near the form; selecting many keys should make the scope of the next apply action explicit.

The original spring graphic can be inspected at `research/ace75/design-reference/original-spring-light.svg` and `original-spring-dark.svg`; these contain the original 55 SVG paths returned by the pure `iL` component, with original theme variables. It is a source reference for a native drawing or asset, not a live measurement. A native threshold illustration must show the actual edited actuation value and physical units; it should not fabricate live key travel when monitoring is off.

### Native accessibility and behavior

The existing native text-contrast adjustments still apply: source half-opacity secondary labels can be raised to the documented accessible colors. Color-coded RT press/release controls need explicit text as well as hue. Key selection needs keyboard focus, pointer feedback and a clear selected state in both themes. The keyboard, profile selector, tabs and apply/discard controls should remain usable at 855×700 and 1440×940. Drafts, read-only mode, hardware confirmation and the separation between static RGB preview and live device state retain their existing behavior.

Dials remain ENERGY 1 / RHYTHM 2 / MOTION 1. The keyboard drawing, grouped settings and a clear selected-key summary provide the visual identity and hierarchy; extra decoration or perpetual animation is unnecessary for this utility.

### Implemented native adaptations in v0.2.1

The case extends into the RT top margin so that the physical button remains attached to the chassis. Keycap depth is limited to a subtle bottom edge; this communicates physical keys without making form cards float. Neutral cap borders are raised to #747B8A (light) / #757C89 (dark) for measured control contrast.

Wide Keys, Trigger and Advanced pages place the diagram beside a bounded inspector so the user can see which keys a setting affects. Trigger uses three tabs rather than a long stack of controls. Narrow Trigger folds the diagram and omits only the auxiliary switch image so the actual editable millimetre slider remains in the first viewport. Selection scope remains visible. The source switch SVG is rasterized in the two official themes and embedded as a native texture; it is an illustration, not simulated travel.

Performance uses functional groups rather than an undifferentiated checkbox list. Full-row native switches keep their title, explanation and state together, and accept Tab/Space with a visible focus ring. Long action palettes wrap into the inspector; DKS phases become a two-column grid instead of overflowing the window. The animation editor keeps its own selection so it cannot silently change the onboard-color editing scope.
