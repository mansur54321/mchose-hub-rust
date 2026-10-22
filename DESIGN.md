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
