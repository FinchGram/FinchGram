# The bird in the menu bar: a dot for unread messages, a hop when one comes

[中文](menu-bar-icon.zh-Hans.md)

Status: a plan, not built. The maintainer asked on 2026-11-03 whether the bird in the menu bar could
be livelier ("你能不能把top menubar上的小鸟弄的生动活波一些，比如，有了新消息，会跳动"), and on 2026-11-04 had
the answer written down for a day they feel like it ("这个不紧急，等我有心情了再说"). The first section is
what is there today; the rest is the plan and the facts it rests on, as of 0.7.9.

## Today

- The icon is an `NSStatusItem` of variable length (`src/platform/macos.rs`, `status_item`), shown
  while Settings → General → "Show in menu bar" is on (on by default since 0.2.13). Its button's
  image is the logo's mark, `ui/logo/svg/FinchGram-mark-16.svg`, drawn by resvg into 36 pixels and
  shown as 18 pt, a template image: macOS colours it for a light or a dark menu bar and for the
  moment the menu is open. Its menu has Open FinchGram and Quit FinchGram, in the UI's language.
  It says nothing about the account: no unread, no connection, no motion.
- What a new message moves today is the Dock icon (`src/telegram/notifications.rs`): the number
  of unread messages, every account's added up, muted chats counted only with "Count muted chats in badge"
  (`updateUnreadMessageCount` of the main chat list, `platform::set_badge`); and one bounce while
  FinchGram is not in front (`platform::flash_icon`, `requestUserAttention`), under Settings →
  Notifications & sounds → "Flash taskbar" (`[notifications] flash_taskbar`, on by default). The
  bounce is asked at the same point as the banners, in `group_changed`: TDLib's notification groups,
  so a muted chat never asks (TDLib makes no notification for it), and neither does the chat the
  user is looking at (FinchGram in front, the window in front, the chat open).
- The connection's state (`updateConnectionState`: waiting for the network, connecting, updating,
  ready) is kept per account in `src/telegram/mod.rs` and shown in the window; nothing outside the
  window shows it.

## What macOS asks of a menu bar extra

Apple's guidelines for the menu bar
(https://developer.apple.com/design/human-interface-guidelines/the-menu-bar, "Menu bar extras",
read 2026-11-03) ask for an image in black and clear, which the system colours so that it looks
right on a light or dark bar and when selected; say the bar is 24 pt high; let people, not the app,
decide whether the extra is shown; and warn that the system hides extras when the bar is short of
room. They say nothing against motion. What follows from that:

- A coloured counter painted into the image (red disc, white number) is out: it stops being a
  template image, and then the app would have to follow the bar's light and dark looks and the
  selected state itself. A dot in black and clear is fine, and widely done.
- Motion is allowed but the bar is shared: nothing may move for long or for ever (Dropbox's icon
  turns only while it syncs). A short hop that ends in a still state is in keeping.
- The user's own choice comes first: "Reduce motion" in the system's accessibility settings
  (`NSWorkspace.accessibilityDisplayShouldReduceMotion`) means no hop, the dot alone.

## The plan

Six points, in the order of their worth.

1. **A dot while there are unread messages.** The most useful part: when the window is closed to
   the Dock, the menu bar is the one place the state can be seen. A filled disc at the top right
   of the bird, with a clear ring around it cut out of the bird so that it reads on the bird's body
   as well as on the bar. It follows the Dock's number exactly: the same sum over the accounts,
   the same "Count muted chats in badge", on at 1, off at 0, so it goes out when the messages are read
   anywhere.
2. **A hop when a message comes.** Up 2–3 pt, a slight squash on landing, a second smaller
   bounce, about half a second in all, eased; then the bird stands still with its dot. Asked at
   the point where the Dock's bounce is asked today, so the rules are already right: no hop for a
   muted chat, none for the chat being looked at, every account's messages count. Unlike the Dock,
   which macOS keeps still while the app is active, the bird hops also while FinchGram is in
   front: that is the liveliness the maintainer asked for (to decide, below). A hop that comes
   while one is running is dropped, and a few seconds pass before the next; a busy group makes one
   hop, not a dance. With "Reduce motion" on, no hop.
3. **Under the switch that exists.** "Flash taskbar" already means "draw my eye with motion when
   a message comes"; the hop goes under it, the dot under none (it is the Dock's number, which has
   no switch either). No new row in Settings: the page follows the design, which drew none for
   this.
4. **Grey while offline (optional).** `appearsDisabled` on the status item's button dims a
   template image the system's way. On while the connection is not ready for more than a few
   seconds, off when it is back: no motion, and the bird tells one more thing. Cheap enough to do
   at the same time, left out if the maintainer finds it fussy.
5. **Not to do.** A wing beat that never stops, or any loop; colour; the number beside the bird as
   the button's title, which takes width in a bar that is often short of it (worth a thought only
   if users ask).
6. **A wing beat needs drawings.** The mark is one silhouette, one path; the wing cannot be moved
   on its own. Two or three frames drawn in Claude Design, as variants of the mark, could be played
   by the same code as the hop; the hop and the dot need no new artwork.

## How it would be built

- **Frames on a timer, the recommended way.** The frames of the hop are the mark drawn by resvg
  as the menu bar image is drawn today, each one with the bird raised by its height at that
  moment and squashed where the curve says, into an image 18 pt wide and 24 pt high (the bar's
  height; the width stays, so the item does not shift its neighbours), at 2× for Retina screens,
  template; the dot is drawn by tiny-skia on top (a clear ring with `BlendMode::Clear`, then the
  disc). A Slint timer on the main thread, where the status item lives, sets the button's image
  every 16 ms for about 36 frames, then the still image with or without its dot. No new crate,
  the template colouring is the system's as today, and a test can read the frames' pixels the way
  the mark's test does.
- **Core Animation, the smoother way.** `wantsLayer` on the status item's button and a
  `CAKeyframeAnimation` of the layer's translation and scale: the GPU eases it, no timer. It needs
  the `objc2-quartz-core` crate (the objc2 family already in Cargo.toml) and the matching feature
  of objc2-app-kit for the layer accessors, and a check by eye that a layer-backed status button
  draws its highlight as before when the menu is open. Worth it only if the frames look stepped,
  which at 60 frames a second they should not.
- **Where it hooks in.** `platform::` grows two calls with empty bodies off macOS, as the badge
  and the bounce have: one for the dot (called from `show_unread_count`, next to the badge) and
  one for the hop (called in `group_changed`, next to the bounce). A third for the connection, if
  point 4 is built, from where `updateConnectionState` is handled. All in all one to two hundred
  lines in `src/platform/macos.rs` and a few in `notifications.rs`.
- **Seeing it.** The picture test (`cargo test screenshots`) draws the window, not the menu bar.
  The status item needs no bundle and no api_id, unlike the notifications, so `cargo run` shows it
  in the real bar; an environment variable for debugging can ask for a hop and a dot to look at.
  The maintainer, who tests the installed release, sees it at the next release.

## To decide

1. Whether to build it, and when.
2. Whether the hop sits under "Flash taskbar" (the plan) or gets a switch of its own.
3. Whether the bird hops while FinchGram is in front (the plan says yes; the Dock cannot).
4. The dot: composed in code as above, or a variant of the mark drawn in Claude Design.
