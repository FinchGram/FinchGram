# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · **العربية**

عميل سطح مكتب مفتوح المصدر لـ Telegram، مع مركز للوسائط، مكتوب بـ Rust و[Slint](https://slint.dev).
يدعم macOS أولًا (Apple silicon)، ثم Windows وLinux لاحقًا.

يستخدم FinchGram واجهة Telegram البرمجية (API) وهو جزء من منظومة Telegram. إنه عميل غير رسمي، ولم
تطوّره Telegram.

الحالة: مرحلة مبكرة. يعمل إنشاء الحساب وتسجيل الدخول وقائمة الدردشات والدردشات بالرسائل النصية، في سمات
التصميم الثلاث: Workbench (الافتراضية) وBroadsheet وTerminal، ويُبدَّل بينها من الإعدادات › المظهر. تظهر
الصور والفيديوهات في الدردشات وتُفتح في عارض يشغّل الفيديو عبر mpv. النقر بزر الفأرة الأيمن على رسالة
يفتح قائمتها: الرد والتعديل والنسخ ونسخ الرابط وإعادة التوجيه والإبلاغ والحذف، أو تحديد عدة رسائل.
ويُدار التحقق بخطوتين من الإعدادات › الخصوصية والأمان. وتأتي بعد ذلك الملفات، والحسابات المتعددة، وفلاتر
الكلمات المفتاحية، والرسائل المجدولة.

التطبيق غلاف (shell). أما Telegram نفسه فتتولاه TDLib، مكتبة Telegram الرسمية، التي تعمل برنامجًا مستقلًا
بجوار الملف التنفيذي: `finchgram-tdlib`، ويبنيه هذا المستودع من مصادر بإصدارات مثبّتة (كما هو الحال مع
ffmpeg في Coova Studio). لا يتواصل الغلاف معه إلا عبر `src/telegram/`، بصيغة JSON الخاصة بـ TDLib. راجع
[docs/architecture.md](../docs/architecture.md) و[docs/conventions.md](../docs/conventions.md) (بالإنجليزية).

كل شيء هنا وعلني: الشيفرة المصدرية، وبناء الاعتماديات (`vendor/`)، والإصدارات.

## التطوير

يتطلب التطوير حاليًا macOS على Apple silicon: إذ لا يُبنى finchgram-tdlib إلا له حتى الآن. سيأتي دعم Linux
وWindows لاحقًا.

لا يوجد finchgram-tdlib ولا libmpv ولا خطوط الواجهة في git: يحتوي `vendor/tdlib/` و`vendor/mpv/` فقط على
السكربتات التي تبنيهما، وتأتي الخطوط من Google Fonts. بعد استنساخ المستودع، احصل عليها مرة واحدة:

```sh
scripts/fetch-tdlib.sh    # ينزّل إصدار finchgram-tdlib المثبّت إلى vendor/tdlib/bin/ ويتحقق من SHA-256
scripts/fetch-mpv.sh      # ينزّل إصدار libmpv المثبّت إلى vendor/mpv/bin/ ويتحقق من SHA-256
scripts/fetch-fonts.sh    # ينزّل خطوط الواجهة المثبّتة إلى vendor/fonts/ ويتحقق من SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- لتعديل finchgram-tdlib نفسه، ابنِه هنا (بضع دقائق؛ يتطلب Xcode أو Command Line Tools،
  و`brew install cmake ninja gperf`، وهي أدوات بناء فقط):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` و`FINCHGRAM_API_HASH`: الخاصان بك، من
  [my.telegram.org](https://my.telegram.org) ← API development tools. يُقرآن عند البناء ولا مكان لهما في
  المستودع أبدًا. من دونهما يعمل التطبيق ويذكر أنه لا يملك API ID.
- يستخدم `FINCHGRAM_TEST_DC=1 cargo run` خوادم الاختبار الخاصة بـ Telegram، ولها حساباتها الخاصة؛ ويحتفظ
  التطبيق لها بقاعدة بيانات منفصلة.
- يرسم `cargo test screenshots -- --ignored` كل صفحة في كل سمة، فاتحة وداكنة، مع دردشات متخيَّلة، في
  `target/screenshots/`: طريقة لرؤية الواجهة من دون حساب.

ينسخ `build.rs` الملف `vendor/tdlib/bin/finchgram-tdlib` بجوار الملف التنفيذي المُصرَّف، فيستخدم `cargo run`
البرنامج نفسه تمامًا الذي يستخدمه التطبيق المُحزَّم؛ ويربط libmpv من `vendor/mpv/bin/` وينسخها هناك أيضًا،
وتُضمَّن الخطوط الموجودة في `vendor/fonts/` داخل الملف التنفيذي. ومن دون أيٍّ منها يفشل البناء برسالة توضّح
ذلك. يتطلب Rust 1.92 أو أحدث.

قاعدة بيانات TDLib والملفات المنزّلة موجودة في `~/Library/Application Support/FinchGram/tdlib/`، والإعدادات
في `~/Library/Application Support/FinchGram/settings.toml`.

## البنية

```
.github/workflows/
  mpv.yml                # يبني libmpv على جهاز نظيف؛ وينشر وسوم mpv-* كإصدارات
  release.yml            # يبني التطبيق مع كل push إلى main؛ وينشر وسوم v* كإصدارات
  tdlib.yml              # يبني finchgram-tdlib على جهاز نظيف؛ وينشر وسوم tdlib-* كإصدارات
Cargo.toml
build.rs                 # يصرّف ui/app.slint، ويضمّن lang/، وينسخ vendor/tdlib/bin/ بجوار الملف التنفيذي
docs/                    # architecture.md و conventions.md (+ zh-Hans)
lang/                    # الترجمات: lang/<الرمز>/LC_MESSAGES/finchgram.po، مُضمَّنة في الملف الثنائي
readme/                  # ملف README هذا بلغات أخرى
release-signing.pub      # المفاتيح العامة المسموح لها بتوقيع الإصدارات؛ مُضمَّنة في التطبيق
scripts/
  bundle.sh              # يبني dist/FinchGram.app (وهو ما يشغّله مسار الإصدار)
  fetch-fonts.sh         # خطوط الواجهة المثبّتة إلى vendor/fonts/
  fetch-mpv.sh           # إصدار libmpv المثبّت إلى vendor/mpv/bin/
  fetch-tdlib.sh         # إصدار finchgram-tdlib المثبّت إلى vendor/tdlib/bin/
  release.sh             # يبدأ إصدارًا: رقم الإصدار، والوسم، والدفع؛ ويتولى GitHub Actions الباقي
src/
  main.rs                # النافذة، والإعدادات، واللغة والسمة، والتحديثات؛ يشغّل Telegram
  fonts.rs               # خطوط الواجهة، مُضمَّنة في الملف التنفيذي
  images.rs              # صور الرسائل، تُفك خارج خيط الواجهة
  telegram/              # الشيفرة الوحيدة التي تتواصل مع finchgram-tdlib
    process.rs           #   يشغّل البرنامج: JSON الخاص بـ TDLib عبر الإدخال والإخراج القياسيين
    api.rs               #   أنواع TDLib التي يستخدمها FinchGram (td_api.tl للإصدار المثبّت)
    mod.rs               #   الطلبات والردود، وإعادة التشغيل؛ تذهب التحديثات إلى store
    store.rs             #   ما قاله TDLib عن الدردشات والمستخدمين والرسائل؛ نماذج الصفحات
    login.rs             #   تسجيل الدخول، وإنشاء الحساب
    chats.rs             #   قائمة الدردشات
    conversation.rs      #   الدردشة المفتوحة: الرسائل، والكتابة
    actions.rs           #   ما يمكن فعله برسالة: قائمتها، الرد، إعادة التوجيه…
    account.rs           #   الملف الشخصي، وتسجيل الخروج
    password.rs          #   التحقق بخطوتين في الإعدادات
    files.rs             #   تنزيل الملفات
    viewer.rs            #   عارض الوسائط: الصور والفيديو والحفظ في «التنزيلات»
    rich_text.rs         #   النص المنسّق: الغامق والمائل والروابط…
  platform/              # ما يختلف من نظام تشغيل إلى آخر
  player/                # الفيديو عبر libmpv، يُرسم في النافذة عبر OpenGL
  update.rs              # التحديث الذاتي: GitHub Releases، والتحقق من التوقيع، والاستبدال، وإعادة التشغيل
  settings.rs            # تفضيلات المستخدم (settings.toml)
  i18n.rs                # لغة الواجهة: الاختيار المحفوظ، وإلا لغة النظام، وإلا الإنجليزية
  screenshots.rs         # كل صفحة مرسومة في ملف PNG (cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs، أداة التوقيع Ed25519 للإصدارات
ui/
  app.slint              # النافذة الرئيسية: القوائم، وأي صفحة تظهر
  state.slint            # حالة التطبيق، يتشاركها Rust والصفحات
  telegram.slint         # ما يعرضه Telegram: الحساب، وتسجيل الدخول، والدردشات، والرسائل
  look.slint             # ألوان السمة وخطوطها وأشكالها، للصفحات المشتركة
  format.slint           # التواريخ والأعداد وأنواع الرسائل بلغة الواجهة
  widgets.slint          # أجزاء صغيرة مشتركة؛ chat.slint: ما تتشاركه نوافذ الدردشة
  viewer.slint           # عارض الوسائط فوق النافذة كلها، بأسلوب كل سمة
  pages/                 # الصفحات التي تتشاركها السمات الثلاث: تسجيل الدخول، والإعدادات، والملف الشخصي
  workbench/             # نافذة الدردشة في سمة Workbench (الافتراضية)
  broadsheet/            # نافذة الدردشة في سمة Broadsheet
  terminal/              # نافذة الدردشة في سمة Terminal
  icons/                 # أيقونات Phosphor (MIT)، عادية وثنائية اللون؛ يسردها icons.slint
  logo/                  # شعار FinchGram (svg وpng) وقواعد استخدامه
vendor/fonts/            # ليس في git: خطوط الواجهة (scripts/fetch-fonts.sh)
vendor/mpv/              # libmpv: ‏mpv وFFmpeg لتشغيل الفيديو
  build.sh               #   يبنيها من مصادر مثبّتة: الإصدارات وSHA-256 في أعلى الملف
  bin/                   #   ليست في git: المكتبة نفسها (scripts/fetch-mpv.sh أو build.sh install)
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   يبنيه من مصادر مثبّتة: commit لـ TDLib، وإصدار OpenSSL وSHA-256 في أعلى الملف
  host/                  #   برنامجنا المضيف الصغير (main.cpp) وملف CMakeLists.txt الخاص به
  bin/                   #   ليس في git: البرنامج الذي يستخدمه التطبيق (scripts/fetch-tdlib.sh أو build.sh install)
  work/, dist/           #   ليسا في git: الملفات الوسيطة والحزمة الناتجة عن بناء محلي
```

## الترجمات

يُكتب كل نص في الواجهة بالشكل `@tr("English text")`. للسلسلة النصية ترجمة واحدة أينما ظهرت (يعطّل build.rs
السياق الافتراضي في Slint)؛ وعندما يحتاج النص الإنجليزي نفسه إلى كلمات مختلفة في موضع آخر، أعطه سياقًا:
`@tr("menu" => "Open")`. استخرج النصوص بالأداة الرسمية، ثم ادمجها في كل لغة:

```sh
cargo install slint-tr-extractor                                          # مرة واحدة
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # يتطلب brew install gettext
```

ثم املأ مدخلات `msgstr` وشغّل `cargo build` من جديد. الإنجليزية هي اللغة المصدر؛ والصينية المبسطة مضمّنة حاليًا.

## الإصدارات

لا يبني الإصدارات إلا GitHub Actions، من commit عليه وسم وعلى جهاز نظيف
(`.github/workflows/release.yml`)، وتُنشر في هذا المستودع: التطبيق مضغوطًا، و`SHA256SUMS`، وتوقيعه
بـ Ed25519. تُحدِّث النسخ المثبّتة نفسها من هنا، ولا تثبّت شيئًا لا تستطيع التحقق منه. يبدأ المشرف
الإصدار بأمر واحد:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1؛ وكذلك minor أو major أو إصدار محدد
```

التوقيع حاليًا ad hoc: عند أول تشغيل لنسخة منزّلة يطلب macOS الإذن مرة واحدة (إعدادات النظام ←
الخصوصية والأمن). أما التحديثات التي يثبّتها التطبيق بنفسه فتعمل دون هذه الخطوة.

## الترخيص

GPL-3.0 ([LICENSE](../LICENSE)). يحتوي finchgram-tdlib على TDLib (Boost Software License 1.0) و
OpenSSL (Apache License 2.0)؛ وخطوط الواجهة مرخّصة بـ SIL Open Font License 1.1، والأيقونات بترخيص MIT.
وترافق نصوص تراخيصها التطبيق.
