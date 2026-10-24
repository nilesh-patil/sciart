//! LIB-R8: what a source scan cannot see, pinned at compile time.
//!
//! The auto traits each public type promises, and every public item named once with its
//! exact signature: removing, renaming or retyping one fails to compile this file.

use std::error::Error as StdError;
use std::fmt::{Debug, Display};
use std::hash::Hash;

use sciart::{Category, Error, ErrorKind, Options, SceneInfo, Stage, Theme};

fn send<T: Send>() {}
fn send_sync_static<T: Send + Sync + 'static>() {}
fn copy_send_sync<T: Copy + Send + Sync>() {}
fn clone_send_sync<T: Clone + Send + Sync>() {}
fn value<T: Debug + PartialEq + Eq + Hash>() {}

#[test]
fn lib_r8_public_types_keep_their_auto_traits() {
    send::<Stage>();
    send_sync_static::<Error>();
    copy_send_sync::<SceneInfo>();
    copy_send_sync::<Category>();
    copy_send_sync::<Theme>();
    copy_send_sync::<ErrorKind>();
    clone_send_sync::<Options>();
    value::<SceneInfo>();
    value::<Category>();
    value::<Theme>();
    value::<ErrorKind>();
    fn error<T: StdError + Display + Debug>() {}
    error::<Error>();
    fn default<T: Default>() {}
    default::<Options>();
    default::<Theme>();
}

#[test]
fn lib_r8_every_public_item_keeps_its_signature() {
    // The catalogue.
    let scenes = sciart::scenes();
    fn exact<I: ExactSizeIterator<Item = SceneInfo>>(i: I) -> usize {
        i.len()
    }
    assert!(exact(scenes) > 0);
    let find: fn(&str) -> Option<SceneInfo> = sciart::find;
    let aurora = find("aurora").unwrap();
    let name: fn(&SceneInfo) -> &'static str = SceneInfo::name;
    let title: fn(&SceneInfo) -> &'static str = SceneInfo::title;
    let description: fn(&SceneInfo) -> &'static str = SceneInfo::description;
    let category: fn(&SceneInfo) -> Category = SceneInfo::category;
    let captions: fn(&SceneInfo) -> &'static [&'static str] = SceneInfo::captions;
    assert_eq!(name(&aurora), "aurora");
    let _ = (title(&aurora), description(&aurora), captions(&aurora));

    let all: fn() -> &'static [Category] = Category::all;
    let id: fn(Category) -> &'static str = Category::id;
    let label: fn(Category) -> &'static str = Category::label;
    assert_eq!(all().len(), 11);
    assert_eq!(
        (id(category(&aurora)), label(Category::EarthSky)),
        ("earth-sky", "Earth & Sky")
    );
    let _ = [
        Category::DeepCosmos,
        Category::SunPlanets,
        Category::EarthSky,
        Category::Voyages,
        Category::ForcesWaves,
        Category::Reactions,
        Category::LivingSystems,
        Category::StatesOfMatter,
        Category::PhysicsOfLife,
        Category::MoleculesOfLife,
        Category::LifeFromMatter,
    ];

    // Themes.
    let themes: fn() -> &'static [Theme] = Theme::all;
    let theme_name: fn(Theme) -> &'static str = Theme::name;
    let from_name: fn(&str) -> Option<Theme> = Theme::from_name;
    assert_eq!(themes().len(), 5);
    assert_eq!(from_name(theme_name(Theme::Aurora)), Some(Theme::Aurora));
    let _ = [
        Theme::Astro,
        Theme::Rainbow,
        Theme::Mono,
        Theme::Aurora,
        Theme::Colorblind,
    ];

    // Options and the stage.
    let seed: fn(Options, u64) -> Options = Options::seed;
    let ascii: fn(Options, bool) -> Options = Options::ascii;
    let show_title: fn(Options, bool) -> Options = Options::title;
    let options = show_title(ascii(seed(Options::default(), 1), false), true);
    let new: fn(&str, u16, u16, Options) -> Result<Stage, Error> = Stage::new;
    let mut stage = new("aurora", 20, 6, options).unwrap();
    let scene: fn(&Stage) -> SceneInfo = Stage::scene;
    let size: fn(&Stage) -> (u16, u16) = Stage::size;
    let time: fn(&Stage) -> f64 = Stage::time;
    let is_failed: fn(&Stage) -> bool = Stage::is_failed;
    let resize: fn(&mut Stage, u16, u16) -> Result<(), Error> = Stage::resize;
    let advance: fn(&mut Stage, f64) -> Result<(), Error> = Stage::advance;
    let render_text: fn(&mut Stage) -> Result<String, Error> = Stage::render_text;
    let render_ansi: fn(&mut Stage, Theme) -> Result<String, Error> = Stage::render_ansi;
    resize(&mut stage, 24, 8).unwrap();
    advance(&mut stage, 0.5).unwrap();
    assert_eq!(
        (scene(&stage), size(&stage), time(&stage), is_failed(&stage)),
        (aurora, (24, 8), 0.5, false)
    );
    assert!(!render_text(&mut stage).unwrap().is_empty());
    assert!(!render_ansi(&mut stage, Theme::Astro).unwrap().is_empty());
    #[cfg(feature = "png")]
    {
        let render_png: fn(&mut Stage, Theme, Vec<u8>) -> Result<(), Error> =
            Stage::render_png::<Vec<u8>>;
        render_png(&mut stage, Theme::Mono, Vec::new()).unwrap();
    }

    // Errors.
    let err = Stage::new("no_such_scene", 1, 1, Options::default()).unwrap_err();
    let kind: fn(&Error) -> ErrorKind = Error::kind;
    let error_scene: fn(&Error) -> Option<&str> = Error::scene;
    assert_eq!(
        (kind(&err), error_scene(&err)),
        (ErrorKind::UnknownScene, Some("no_such_scene"))
    );
    assert!(err.source().is_none());
    let _ = [
        ErrorKind::UnknownScene,
        ErrorKind::ScenePanicked,
        ErrorKind::InvalidInput,
        ErrorKind::Io,
    ];
}

#[cfg(feature = "widget")]
#[test]
fn lib_r8_the_widget_keeps_its_signature() {
    use ratatui::widgets::Widget;
    use sciart::SceneWidget;

    fn widget<W: Widget + Debug>(_: &W) {}
    // The widget's lifetime is the impl's, so the signatures are pinned for one lifetime.
    fn pinned<'a>(stage: &'a mut Stage) -> SceneWidget<'a> {
        let new: fn(&'a mut Stage) -> SceneWidget<'a> = SceneWidget::new;
        let theme: fn(SceneWidget<'a>, Theme) -> SceneWidget<'a> = SceneWidget::theme;
        theme(new(stage), Theme::Astro)
    }
    let mut stage = Stage::new("starfield", 4, 4, Options::default()).unwrap();
    widget(&pinned(&mut stage));
}

#[cfg(feature = "unstable-generator")]
#[test]
fn lib_r8_the_experimental_entry_point_exists() {
    use sciart::experimental::Generator;

    type Scene = Box<dyn Generator>;
    type FromGenerator = fn(Scene, u16, u16, Options) -> Result<Stage, Error>;
    let from_generator: FromGenerator = Stage::from_generator;
    let _ = from_generator;
}
