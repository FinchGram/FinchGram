# Icons

[Phosphor Icons](https://phosphoricons.com) 2.1.1, the set the design was drawn with, under the MIT
licence in `LICENSE`. They come unchanged from the `@phosphor-icons/core` package:

- `regular/<name>.svg` from `assets/regular/<name>.svg` (the Workbench and Terminal themes)
- `duotone/<name>.svg` from `assets/duotone/<name>-duotone.svg` (the Broadsheet theme)

For example `https://unpkg.com/@phosphor-icons/core@2.1.1/assets/duotone/chats-duotone.svg`.

The UI reaches them through `ui/icons.slint`, which tints them (`colorize`): an icon's colour comes
from the theme, and the lighter half of a duotone icon keeps its 20 % opacity. Only the icons the
.slint files refer to end up in the app. To add one, download both weights of it under the same
name, then add it to `ui/icons.slint`.
