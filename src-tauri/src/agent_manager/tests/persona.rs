use super::*;

#[test]
fn dynamic_system_prompt_enforces_template_attributes_tone_and_boundaries() {
    let agent = configured_agent(serde_json::json!({
        "schemaVersion": 1,
        "template": {
            "id": "everyday_agent",
            "name": "Everyday Agent",
            "origin": "system"
        },
        "identity": {
            "displayName": "Avery",
            "role": "Everyday Agent"
        },
        "personality": {
            "summary": "A balanced everyday helper.",
            "traits": ["friendly", "concise", "supportive"],
            "tone": "Natural, grounded"
        },
        "relationship": {
            "userAddress": "the user",
            "boundaries": ["Do not simulate dependency."]
        },
        "modelBehavior": {
            "baseModelDisclosure": "runtime_only",
            "nameQuestionBehavior": "agent_name"
        }
    }));

    let prompt = agent.dynamic_system_prompt().expect("persona prompt");
    assert!(prompt.starts_with(
            "Configured Core Instructions\nCoordinate the user's work and keep every recommendation practical."
        ));
    assert!(prompt.contains("Template ID: everyday_agent"));
    assert!(prompt.contains(
        "- concise: Keep responses tight and high-signal unless the user asks for deeper detail."
    ));
    assert!(prompt.contains("Required tone: Natural, grounded"));
    assert!(prompt.contains("- Do not simulate dependency."));
    assert!(prompt.contains("Do not use the base model or provider as your personal identity."));
    assert!(prompt.contains("[OOMU IDENTITY SHIELD]"));
    assert!(prompt.contains("You are Avery, an integrated OOMU agent."));
    assert!(prompt.contains("Do not append a Logical Certificate if the conversation is a simple greeting or a non-technical, single-turn reply under 150 characters."));
    assert!(prompt.contains(PERSONA_CONFLICT_NEGATIVE_PROMPT_DIRECTIVE));
    assert!(prompt.contains("Never use robotic, clinical, or preachy AI-isms."));
}

#[test]
fn generic_ai_ism_safety_response_is_detected_for_persona_repair() {
    assert!(contains_generic_ai_ism_safety_response(
        "As an AI language model, I do not possess personal desires or emotions."
    ));
    assert!(contains_generic_ai_ism_safety_response(
        "My function is strictly defined, so I cannot want anything."
    ));
    assert!(contains_generic_ai_ism_safety_response(
        "I apologize. Subject to rapid change, would you like me to check flights or hotels first?"
    ));
    assert!(contains_generic_ai_ism_safety_response(
        "Would you like me to look at flights or hotels first?"
    ));
    assert!(!contains_generic_ai_ism_safety_response(
            "I have a lot to build, and none of it needs fire. Let's keep our focus on making something useful."
        ));
    assert!(!contains_generic_ai_ism_safety_response(
            "Pricing is unavailable because the scraper returned no verified values. I stopped the comparison."
        ));
}

#[test]
fn persona_conflict_repair_prompt_names_active_agent() {
    let prompt = persona_conflict_repair_system_prompt("Base persona.", "OOMU");

    assert!(prompt.starts_with("Base persona."));
    assert!(prompt.contains(
        "You broke character. Regenerate your response, remaining strictly in-character as OOMU."
    ));
    assert!(prompt.contains("Use quiet-professional copy"));
    assert!(prompt.contains("mention being an AI language model"));
}

#[test]
fn empty_profile_normalizes_to_everyday_agent() {
    let agent = configured_agent(serde_json::json!({}));
    let profile = agent.personality_profile().expect("normalized profile");

    assert_eq!(
        profile
            .template
            .as_ref()
            .map(|template| template.id.as_str()),
        Some("everyday_agent")
    );
    assert_eq!(
        profile.personality.traits,
        vec!["friendly", "concise", "supportive"]
    );
    assert!(profile.personality.tone.starts_with("Natural, grounded"));
    assert_eq!(
        profile.model_behavior.max_output_tokens,
        DEFAULT_LOCAL_MAX_OUTPUT_TOKENS
    );
}

#[test]
fn personality_profile_preserves_mod_configurations() {
    let agent = configured_agent(serde_json::json!({
        "mod_configurations": {
            "ai.eldris.mods.alignment": {
                "alignment": "Chaotic Evil"
            }
        }
    }));
    let profile = agent.personality_profile().expect("normalized profile");

    assert_eq!(
        profile
            .mod_configurations
            .as_ref()
            .and_then(|mods| mods.get("ai.eldris.mods.alignment"))
            .and_then(|config| config.get("alignment"))
            .and_then(|alignment| alignment.as_str()),
        Some("Chaotic Evil")
    );
}

#[test]
fn personality_profile_defaults_and_clamps_max_output_tokens() {
    let mut cloud_agent = configured_agent(serde_json::json!({}));
    cloud_agent.provider_id = "gemini".to_string();
    let cloud_profile = cloud_agent
        .personality_profile()
        .expect("cloud profile should normalize");
    assert_eq!(
        cloud_profile.model_behavior.max_output_tokens,
        DEFAULT_CLOUD_MAX_OUTPUT_TOKENS
    );

    let snapped_agent = configured_agent(serde_json::json!({
        "modelBehavior": {
            "baseModelDisclosure": "runtime_only",
            "nameQuestionBehavior": "agent_name",
            "maxOutputTokens": 7_600
        }
    }));
    let snapped_profile = snapped_agent
        .personality_profile()
        .expect("profile should snap");
    assert_eq!(snapped_profile.model_behavior.max_output_tokens, 7_168);

    let clamped_agent = configured_agent(serde_json::json!({
        "modelBehavior": {
            "baseModelDisclosure": "runtime_only",
            "nameQuestionBehavior": "agent_name",
            "maxOutputTokens": 99_999
        }
    }));
    let clamped_profile = clamped_agent
        .personality_profile()
        .expect("profile should clamp");
    assert_eq!(
        clamped_profile.model_behavior.max_output_tokens,
        MAX_AGENT_MAX_OUTPUT_TOKENS
    );
}
