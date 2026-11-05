use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HarnessInputModality {
    Text,
    Image,
    Audio,
}

pub(crate) fn harness_input_modalities(source: &[String]) -> Vec<HarnessInputModality> {
    let mut result = Vec::new();
    for modality in source {
        let supported = match modality.as_str() {
            "text" => HarnessInputModality::Text,
            "image" => HarnessInputModality::Image,
            "audio" => HarnessInputModality::Audio,
            _ => continue,
        };
        if !result.contains(&supported) {
            result.push(supported);
        }
    }
    result
}

pub(crate) fn validate_text_model(model_id: &str, source: &[String]) -> Result<(), String> {
    if !harness_input_modalities(source).contains(&HarnessInputModality::Text) {
        return Err(format!(
            "ai_model_capability_incompatible:model={model_id};field=input_modalities;text_required"
        ));
    }
    Ok(())
}

pub(crate) fn validate_visual_attachments<'a>(
    source: &[String],
    media_types: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let supports_image = harness_input_modalities(source).contains(&HarnessInputModality::Image);
    for media_type in media_types {
        if !matches!(media_type, "video" | "image" | "document") {
            return Err("ai_attachment_not_supported".to_owned());
        }
        if media_type == "video" && !supports_image {
            return Err("ai_video_input_not_supported".to_owned());
        }
        if media_type != "document" && !supports_image {
            return Err("ai_attachment_not_supported".to_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        HarnessInputModality, harness_input_modalities, validate_text_model,
        validate_visual_attachments,
    };

    fn modalities(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn platform_video_capability_remains_intact_while_harness_receives_text() {
        let source = modalities(&["text", "video"]);
        assert_eq!(
            harness_input_modalities(&source),
            vec![HarnessInputModality::Text]
        );
        assert_eq!(source, modalities(&["text", "video"]));
        assert!(validate_text_model("minimax", &source).is_ok());
        assert_eq!(
            validate_visual_attachments(&source, ["video"]),
            Err("ai_video_input_not_supported".to_owned())
        );
    }

    #[test]
    fn video_frames_require_real_image_capability_without_adding_it() {
        let source = modalities(&["text", "image", "video", "audio", "image"]);
        assert_eq!(
            harness_input_modalities(&source),
            vec![
                HarnessInputModality::Text,
                HarnessInputModality::Image,
                HarnessInputModality::Audio
            ]
        );
        assert!(validate_visual_attachments(&source, ["image", "video", "document"]).is_ok());
        assert_eq!(
            validate_visual_attachments(&modalities(&["text", "video"]), ["image"]),
            Err("ai_attachment_not_supported".to_owned())
        );
    }

    #[test]
    fn unsupported_source_capabilities_cannot_create_a_text_capability() {
        let source = modalities(&["video", "future-input"]);
        assert!(harness_input_modalities(&source).is_empty());
        assert!(
            validate_text_model("video-only", &source)
                .unwrap_err()
                .contains("model=video-only;field=input_modalities")
        );
        assert!(validate_visual_attachments(&source, ["document"]).is_ok());
        assert!(serde_json::from_str::<HarnessInputModality>("\"video\"").is_err());
    }
}
