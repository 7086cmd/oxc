use oxc_ast::{
    ast::{
        Expression, JSXAttributeItem, JSXAttributeName, JSXAttributeValue, JSXChild,
        JSXElementName, JSXExpression, JSXOpeningElement,
    },
    AstKind,
};
use oxc_diagnostics::{
    miette::{self, Diagnostic},
    thiserror::Error,
};
use oxc_macros::declare_oxc_lint;
use oxc_span::Span;
use rustc_hash::FxHasher;
use std::{collections::HashMap, hash::BuildHasherDefault};

use crate::{context::LintContext, rule::Rule, utils::has_jsx_prop, AstNode};

#[derive(Debug, Error, Diagnostic)]
#[error("eslint-plugin-jsx-a11y(media-has-caption): Missing <track> element with captions inside <audio> or <video> element")]
#[diagnostic(
    severity(warning),
    help("Media elements such as <audio> and <video> must have a <track> for captions.")
)]
struct MediaHasCaptionDiagnostic(#[label] pub Span);

#[derive(Debug, Default, Clone)]
pub struct MediaHasCaption(Box<MediaHasCaptionConfig>);

#[derive(Debug, Clone)]
pub struct MediaHasCaptionConfig {
    audio: Vec<String>,
    video: Vec<String>,
    track: Vec<String>,
}

impl Default for MediaHasCaptionConfig {
    fn default() -> Self {
        Self {
            audio: vec!["audio".to_string()],
            video: vec!["video".to_string()],
            track: vec!["track".to_string()],
        }
    }
}

declare_oxc_lint!(
    /// ### What it does
    /// Checks if `<audio>` and `<video>` elements have a `<track>` element for captions.
    /// This ensures media content is accessible to all users, including those with hearing impairments.
    ///
    /// ### Why is this bad?
    /// Without captions, audio and video content is not accessible to users who are deaf or hard of hearing.
    /// Captions are also useful for users in noisy environments or where audio is not available.
    ///
    /// ### Example
    /// ```jsx
    /// // Good
    /// <audio><track kind="captions" src="caption_file.vtt" /></audio>
    /// <video><track kind="captions" src="caption_file.vtt" /></video>
    ///
    /// // Bad
    /// <audio></audio>
    /// <video></video>
    /// ```
    MediaHasCaption,
    correctness
);

impl Rule for MediaHasCaption {
    fn from_configuration(value: serde_json::Value) -> Self {
        let mut config = MediaHasCaptionConfig::default();

        if let Some(rule_config) = value.as_object() {
            if let Some(audio) = rule_config.get("audio").and_then(|v| v.as_array()) {
                config.audio.extend(audio.iter().filter_map(|v| v.as_str().map(String::from)));
            }
            if let Some(video) = rule_config.get("video").and_then(|v| v.as_array()) {
                config.video.extend(video.iter().filter_map(|v| v.as_str().map(String::from)));
            }
            if let Some(track) = rule_config.get("track").and_then(|v| v.as_array()) {
                config.track.extend(track.iter().filter_map(|v| v.as_str().map(String::from)));
            }
        }

        Self(Box::new(config))
    }
    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        let jsx_a11y_settings = ctx.settings().jsx_a11y;
        let customed_components = jsx_a11y_settings.components;
        let polymorphic_prop_name = &jsx_a11y_settings.polymorphic_prop_name;
        let AstKind::JSXOpeningElement(jsx_el) = node.kind() else { return };

        let element_name =
            get_mapped_element_name(jsx_el, &customed_components, polymorphic_prop_name.as_ref());

        let is_audio_or_video =
            self.0.audio.contains(&element_name) || self.0.video.contains(&element_name);

        // Bail out if the element is not an <audio /> or <video /> element.
        if !is_audio_or_video {
            return;
        }

        let muted = jsx_el.attributes.iter().any(|attr_item| {
            if let JSXAttributeItem::Attribute(attr) = attr_item {
                if let JSXAttributeName::Identifier(iden) = &attr.name {
                    if iden.name == "muted" {
                        return match &attr.value {
                            Some(JSXAttributeValue::ExpressionContainer(exp)) => {
                                match &exp.expression {
                                    JSXExpression::Expression(Expression::BooleanLiteral(
                                        boolean,
                                    )) => boolean.value,
                                    _ => false,
                                }
                            }
                            Some(JSXAttributeValue::StringLiteral(lit)) => lit.value == "true",
                            None => true, // e.g. <video muted></video>
                            _ => false,
                        };
                    }
                }
            }
            false
        });

        // Bail out if the element is muted as captions are not required for muted media. (e.g <video muted />)
        if muted {
            return;
        }

        let parent = ctx.nodes().parent_node(node.id()).and_then(|parent_node| {
            if let AstKind::JSXElement(jsx_element) = parent_node.kind() {
                Some(jsx_element)
            } else {
                None
            }
        });

        let has_caption = parent.map_or(false, |parent| {
            if parent.children.is_empty() {
                ctx.diagnostic(MediaHasCaptionDiagnostic(parent.opening_element.span));
                return false;
            }

            parent.children.iter().any(|child| match child {
                JSXChild::Element(child_el) => {
                    let child_name = get_mapped_element_name(
                        &child_el.opening_element,
                        &customed_components,
                        polymorphic_prop_name.as_ref(),
                    );
                    self.0.track.contains(&child_name)
                        && child_el.opening_element.attributes.iter().any(|attr| {
                            if let JSXAttributeItem::Attribute(attr) = attr {
                                if let JSXAttributeName::Identifier(iden) = &attr.name {
                                    if let Some(JSXAttributeValue::StringLiteral(s)) = &attr.value {
                                        return iden.name == "kind"
                                            && s.value.to_lowercase() == "captions";
                                    }
                                }
                            }
                            false
                        })
                }
                _ => false,
            })
        });

        let span = parent.map_or(jsx_el.span, |parent| parent.span);

        if !has_caption {
            ctx.diagnostic(MediaHasCaptionDiagnostic(span));
        }
    }
}

/// Get the value of the `as` prop if it exists.
fn get_polymorphic_element_name(
    jsx_el: &JSXOpeningElement<'_>,
    polymorphic_prop_name: &str,
) -> Option<String> {
    has_jsx_prop(jsx_el, polymorphic_prop_name).and_then(|attr_item| match attr_item {
        JSXAttributeItem::Attribute(attr) => match &attr.value {
            Some(JSXAttributeValue::StringLiteral(lit)) => Some(lit.value.to_string()),
            _ => None,
        },
        JSXAttributeItem::SpreadAttribute(_) => None,
    })
}

/// Get the name of the element, taking into account custom components and polymorphic components.
fn get_mapped_element_name(
    jsx_el: &JSXOpeningElement,
    customed_components: &HashMap<String, String, BuildHasherDefault<FxHasher>>,
    polymorphic_prop_name: Option<&String>,
) -> String {
    let JSXElementName::Identifier(iden) = &jsx_el.name else { return String::new() };
    let element_name = polymorphic_prop_name.map_or_else(
        || iden.name.to_string(),
        |polymorphic_name| {
            get_polymorphic_element_name(jsx_el, polymorphic_name)
                .unwrap_or_else(|| iden.name.to_string())
        },
    );
    customed_components.get(&element_name).unwrap_or(&element_name).to_string()
}

#[test]
fn test() {
    use crate::tester::Tester;

    fn config() -> serde_json::Value {
        serde_json::json!({
            "audio": [ "Audio" ],
            "video": [ "Video" ],
            "track": [ "Track" ],
        })
    }

    fn settings() -> serde_json::Value {
        serde_json::json!({
            "jsx-a11y": {
            "polymorphicPropName": "as",
            "components": {
                "Audio": "audio",
                "Video": "video",
                "Track": "track",
                },
            }
        })
    }

    let pass = vec![
        (r"<div />;", None, None),
        (r"<MyDiv />;", None, None),
        (r"<audio><track kind='captions' /></audio>", None, None),
        (r"<audio><track kind='Captions' /></audio>", None, None),
        (r"<audio><track kind='Captions' /><track kind='subtitles' /></audio>", None, None),
        (r"<video><track kind='captions' /></video>", None, None),
        (r"<video><track kind='Captions' /></video>", None, None),
        (r"<video><track kind='Captions' /><track kind='subtitles' /></video>", None, None),
        (r"<audio muted={true}></audio>", None, None),
        (r"<video muted={true}></video>", None, None),
        (r"<video muted></video>", None, None),
        (r"<Audio><track kind='captions' /></Audio>", Some(config()), None),
        (r"<audio><Track kind='captions' /></audio>", Some(config()), None),
        (r"<Video><track kind='captions' /></Video>", Some(config()), None),
        (r"<video><Track kind='captions' /></video>", Some(config()), None),
        (r"<Audio><Track kind='captions' /></Audio>", Some(config()), None),
        (r"<Video><Track kind='captions' /></Video>", Some(config()), None),
        (r"<Video muted></Video>", Some(config()), None),
        (r"<Video muted={true}></Video>", Some(config()), None),
        (r"<Audio muted></Audio>", Some(config()), None),
        (r"<Audio muted={true}></Audio>", Some(config()), None),
        (r"<Audio><track kind='captions' /></Audio>", None, Some(settings())),
        (r"<audio><Track kind='captions' /></audio>", None, Some(settings())),
        (r"<Video><track kind='captions' /></Video>", None, Some(settings())),
        (r"<video><Track kind='captions' /></video>", None, Some(settings())),
        (r"<Audio><Track kind='captions' /></Audio>", None, Some(settings())),
        (r"<Video><Track kind='captions' /></Video>", None, Some(settings())),
        (r"<Video muted></Video>", None, Some(settings())),
        (r"<Video muted={true}></Video>", None, Some(settings())),
        (r"<Audio muted></Audio>", None, Some(settings())),
        (r"<Audio muted={true}></Audio>", None, Some(settings())),
        (r"<Box as='audio' muted={true}></Box>", None, Some(settings())),
    ];

    let fail = vec![
        (r"<audio><track /></audio>", None, None),
        (r"<audio><track kind='subtitles' /></audio>", None, None),
        (r"<audio />", None, None),
        (r"<video><track /></video>", None, None),
        (r"<video><track kind='subtitles' /></video>", None, None),
        (r"<Audio muted={false}></Audio>", Some(config()), None),
        (r"<Video muted={false}></Video>", Some(config()), None),
        (r"<Audio muted={false}></Audio>", None, Some(settings())),
        (r"<Video muted={false}></Video>", None, Some(settings())),
        (r"<video />", None, None),
        (r"<audio>Foo</audio>", None, None),
        (r"<video>Foo</video>", None, None),
        (r"<Audio />", Some(config()), None),
        (r"<Video />", Some(config()), None),
        (r"<Audio />", None, Some(settings())),
        (r"<Video />", None, Some(settings())),
        (r"<audio><Track /></audio>", Some(config()), None),
        (r"<video><Track /></video>", Some(config()), None),
        (r"<Audio><Track kind='subtitles' /></Audio>", Some(config()), None),
        (r"<Video><Track kind='subtitles' /></Video>", Some(config()), None),
        (r"<Audio><Track kind='subtitles' /></Audio>", None, Some(settings())),
        (r"<Video><Track kind='subtitles' /></Video>", None, Some(settings())),
        (r"<Box as='audio'><Track kind='subtitles' /></Box>", None, Some(settings())),
    ];

    Tester::new_with_settings(MediaHasCaption::NAME, pass, fail).test_and_snapshot();
}
