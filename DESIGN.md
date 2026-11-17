# DESIGN.md: TetherControl landing page

## Direction (owner-selected)

- **Product:** TetherControl, a LAN tool that presents a Linux monitor as a macOS virtual display.
- **Personality:** field manual. Plain, exact, useful. No marketing voice.
- **Visual language:** off-white paper, near-black ink, hairline rules. Monospace is reserved for literal values such as commands, ports, paths and coordinates.
- **Palette:** paper `#F4F1EA`, ink `#1C1C1A`, graphite `#5F5F58`, rule `#8C877C`, one signal color `#B4461F`.
- **Typography:** IBM Plex Sans for prose and headings, IBM Plex Mono for commands, code paths and figure labels.
- **Mood:** a reference page someone can follow while setting up two machines.
- **Dials:** ENERGY 1 / RHYTHM 2 / MOTION 1.
- **Hero visual:** a labeled schematic diagram, marked as a diagram rather than a screenshot.
- **Design Read:** reading this as a developer setup reference for someone with a MacBook and a Linux PC, in a field-manual visual language, dials ENERGY 1 / RHYTHM 2 / MOTION 1.

## Decision record (one line per major decision)

- Paper background: the page is read like setup documentation, not a product advertisement.
- Ink and graphite only for text: separates content from the single accent without adding a palette.
- Signal color on the primary action and the diagram's boundary line only: two uses keep it a signal.
- IBM Plex Sans: drawn for technical communication and legible at small sizes.
- IBM Plex Mono for literal values only: commands and coordinates are data, so they get a data face.
- No badge above the headline: the headline already states the subject.
- No icon set: the content is commands and behavior rules, so icons would add no information.
- Two numbered setup blocks instead of a feature grid: the reader has two machines to configure, in order.
- Definition list for behavior: each row is a condition and its result, which a table-like list states directly.
- Hairline rules between sections: separates content without shadows or elevation.
- No motion beyond hover and focus: MOTION 1, so movement would contradict the declared dial.
- Single column below 640px: commands and definitions need full width to remain readable.
- Three width states (single column, paired blocks, full two-column): the layout reflows where content stops fitting, not at device widths.
