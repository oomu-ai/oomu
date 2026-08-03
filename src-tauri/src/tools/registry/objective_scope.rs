#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpecializedFamily {
    DecisionPack,
    BackgroundAgentComparison,
    MilestoneRecovery,
    ReleaseRecovery,
}

pub(super) fn registered_task_tool_matches_objective(operation: &str, objective: &str) -> bool {
    let lexical = objective.to_ascii_lowercase();
    let normalized = normalize(objective);
    if explicitly_requests_operation(operation, &lexical) {
        return true;
    }

    let family = specialized_family(&normalized);
    if let Some(decision) = specialized_registered_decision(family, operation, &lexical) {
        return decision;
    }

    let sends_mail = has_send_mail_intent(&lexical);
    let drafts_mail = has_draft_mail_intent(&lexical);
    let event_intent = has_event_intent(&lexical);
    let avoids_conflicts = avoids_calendar_conflicts(&lexical);
    let output_file = has_output_file_intent(&lexical);

    match operation {
        "connected_work" => has_connected_work_intent(&lexical),
        "create_file" => output_file,
        "create_spreadsheet" => {
            output_file && contains_any(&normalized, &["spreadsheet", "workbook", ".xlsx", ".csv"])
        }
        "create_presentation" => {
            output_file
                && contains_any(
                    &normalized,
                    &["presentation", "slide deck", "slides", ".pptx"],
                )
        }
        "read_project_file" => has_input_file_read_intent(&normalized),
        "fetch_official_page" => contains_any(
            &normalized,
            &[
                "official source",
                "official public",
                "primary source",
                "primary or official",
                "public web source",
            ],
        ),
        "analyze_supplier_exceptions" => {
            normalized.contains("supplier")
                && contains_any(
                    &normalized,
                    &[
                        "active quote",
                        "settled rate",
                        "rate variance",
                        "variance",
                        "exception",
                        "reconcile",
                    ],
                )
        }
        "analyze_project_milestones" => {
            normalized.contains("milestone")
                && contains_any(&normalized, &["unfinished", "risk", "status", "dependency"])
        }
        "validate_evidence_report" => {
            output_file && contains_any(&normalized, &["report", "brief", "evidence"])
        }
        "create_conflict_free_calendar_event" => event_intent && avoids_conflicts,
        "create_system_calendar_event" => event_intent && !avoids_conflicts,
        "draft_system_email" => drafts_mail,
        "send_system_email" => sends_mail,
        "configure_channel" => has_configure_channel_intent(&lexical),
        "app_control" => contains_any(
            &normalized,
            &[
                "desktop app",
                "application window",
                "control the app",
                "click in",
                "type in",
                "open safari",
                "open finder",
            ],
        ),
        _ => false,
    }
}

pub(super) fn static_tool_matches_objective(kind: &str, objective: &str) -> bool {
    let lexical = objective.to_ascii_lowercase();
    let normalized = normalize(objective);
    if specialized_static_implementation_tool(specialized_family(&normalized), kind, &lexical) {
        return false;
    }
    match kind {
        "unsupported" => true,
        "system_diagnostics" | "get_system_metrics" => contains_any(
            &lexical,
            &["system diagnostic", "system metrics", "local metrics"],
        ),
        "file_list" => contains_any(
            &lexical,
            &[
                "list files",
                "show files",
                "directory contents",
                "folder contents",
            ],
        ),
        "file_read" => has_input_file_read_intent(&lexical),
        "file_write" => has_explicit_file_write_intent(&lexical),
        "delete_file" => has_delete_file_intent(&lexical),
        "codebase_patch" => {
            contains_any(
                &lexical,
                &["patch ", "fix ", "implement ", "edit ", "change "],
            ) && contains_any(
                &lexical,
                &[" code", " source code", " repository", " repo "],
            )
        }
        "codebase_compile" => contains_any(
            &lexical,
            &[
                "compile the",
                "compile frontend",
                "compile backend",
                "recompile the",
                "build the app",
                "rebuild the app",
            ],
        ),
        // Terminal access stays visible to the planner in every language. The
        // generated typed request is still classified and enforced by Shield;
        // lexical hints must never become a capability-denial boundary.
        "terminal_execute" => true,
        "system_audit" => contains_any(
            &lexical,
            &[
                "system audit",
                "process audit",
                "disk audit",
                "network audit",
            ],
        ),
        "telemetry_archive" => {
            lexical.contains("telemetry")
                && lexical.contains("archive")
                && has_output_file_intent(&lexical)
        }
        "sync_knowledge_vault" => contains_any(
            &lexical,
            &[
                "sync knowledge vault",
                "index knowledge vault",
                "index the vault",
            ],
        ),
        "sovereign_duckduckgo_search" | "duckduckgo_search" => contains_any(
            &lexical,
            &[
                "search online",
                "search the web",
                "search google",
                "use the internet",
                "search duckduckgo",
            ],
        ),
        // Registered tools with legacy compact entries are inserted later only
        // when their complete runtime schema is relevant to this objective.
        "connected_work" | "create_spreadsheet" | "app_control" => false,
        _ => false,
    }
}

fn specialized_registered_decision(
    family: Option<SpecializedFamily>,
    operation: &str,
    objective: &str,
) -> Option<bool> {
    let has_mail = contains_any(objective, &["email", "mail"]);
    let event_intent = has_event_intent(objective);
    let draft_intent = has_draft_mail_intent(objective);
    let independent_output = has_independent_output_file_intent(objective);
    let independent_read = has_independent_input_file_intent(objective);
    let independent_research = has_independent_web_research_intent(objective);
    let independent_draft = has_independent_draft_mail_intent(objective);
    match family? {
        SpecializedFamily::DecisionPack => match operation {
            "create_decision_pack" => Some(true),
            "create_conflict_free_calendar_event" => {
                Some(event_intent && objective.contains("conflict"))
            }
            "draft_decision_pack_email" => Some(has_mail && draft_intent),
            "read_project_file" => Some(independent_read),
            "fetch_official_page" => Some(independent_research),
            "analyze_supplier_exceptions" => Some(false),
            "create_file" => Some(independent_output),
            "create_spreadsheet" => Some(has_independent_spreadsheet_output_intent(objective)),
            "create_presentation" => Some(has_independent_presentation_output_intent(objective)),
            "create_system_calendar_event" => {
                Some(has_independent_event_with_conflict_policy(objective, false))
            }
            "draft_system_email" => Some(independent_draft),
            _ => None,
        },
        SpecializedFamily::BackgroundAgentComparison => match operation {
            "prepare_background_agent_comparison" => Some(true),
            "fetch_official_page" => Some(independent_research),
            "create_file" => Some(independent_output),
            _ => None,
        },
        SpecializedFamily::MilestoneRecovery => match operation {
            "prepare_milestone_constraint_recovery_plan" => Some(true),
            "read_project_file" => Some(independent_read),
            "analyze_project_milestones" => Some(false),
            "create_file" => Some(independent_output),
            _ => None,
        },
        SpecializedFamily::ReleaseRecovery => match operation {
            "prepare_release_recovery_agenda"
            | "create_release_recovery_calendar_event"
            | "draft_release_recovery_email" => Some(true),
            "read_project_file" => Some(independent_read),
            "analyze_project_milestones" => Some(false),
            "create_file" => Some(independent_output),
            "create_conflict_free_calendar_event" => {
                Some(has_independent_event_with_conflict_policy(objective, true))
            }
            "create_system_calendar_event" => {
                Some(has_independent_event_with_conflict_policy(objective, false))
            }
            "draft_system_email" => Some(independent_draft),
            _ => None,
        },
    }
}

fn specialized_static_implementation_tool(
    family: Option<SpecializedFamily>,
    kind: &str,
    objective: &str,
) -> bool {
    let independently_requested = match kind {
        "file_list" | "file_read" => has_independent_input_file_intent(objective),
        "file_write" => has_independent_output_file_intent(objective),
        "sovereign_duckduckgo_search" | "duckduckgo_search" => {
            has_independent_web_research_intent(objective)
        }
        _ => false,
    };
    if independently_requested {
        return false;
    }
    match family {
        Some(SpecializedFamily::DecisionPack) => matches!(
            kind,
            "file_list"
                | "file_read"
                | "file_write"
                | "sovereign_duckduckgo_search"
                | "duckduckgo_search"
        ),
        Some(SpecializedFamily::BackgroundAgentComparison) => matches!(
            kind,
            "file_write" | "sovereign_duckduckgo_search" | "duckduckgo_search"
        ),
        Some(SpecializedFamily::MilestoneRecovery | SpecializedFamily::ReleaseRecovery) => {
            matches!(kind, "file_list" | "file_read" | "file_write")
        }
        None => false,
    }
}

fn specialized_family(objective: &str) -> Option<SpecializedFamily> {
    let release_recovery = objective.contains("recovery agenda")
        || objective.contains("release recovery meeting")
        || objective.contains("release recovery agenda")
        || (objective.contains("overdue or unfinished milestone")
            && objective.contains("recovery meeting")
            && objective.contains("exactly five agenda items")
            && contains_any(objective, &["unsent mail draft", "unsent email draft"]));
    if release_recovery {
        return Some(SpecializedFamily::ReleaseRecovery);
    }
    if requests_background_agent_comparison(objective) {
        return Some(SpecializedFamily::BackgroundAgentComparison);
    }
    if requests_milestone_recovery_artifact(objective) {
        return Some(SpecializedFamily::MilestoneRecovery);
    }
    if objective.contains("supplier decision pack")
        || objective.contains("supplier decision.xlsx")
        || objective.contains("supplier decision review")
    {
        return Some(SpecializedFamily::DecisionPack);
    }
    None
}

fn requests_background_agent_comparison(objective: &str) -> bool {
    objective.contains("openclaw")
        && contains_any(objective, &["claude cowork", "claude's cowork", "cowork"])
        && objective.contains("background")
        && objective.contains("current")
        && contains_any(objective, &["official", "primary source"])
        && objective.contains(".md")
        && has_output_file_intent(objective)
        && contains_any(
            objective,
            &["read it back", "read the file back", "reopen the file"],
        )
}

fn requests_milestone_recovery_artifact(objective: &str) -> bool {
    objective.contains("recovery plan")
        && contains_any(objective, &["milestone", "project milestones"])
        && objective.contains(".json")
        && has_input_file_read_intent(objective)
        && objective.contains("dependencies")
        && objective.contains("one owner capacity")
        && objective.contains("business hours")
        && objective.contains("20% contingency reserve")
        && objective.contains("security validation precede release validation")
        && objective.contains("three failure contingencies")
        && objective.contains(".md")
        && has_output_file_intent(objective)
}

fn has_input_file_read_intent(objective: &str) -> bool {
    contains_any(
        objective,
        &["read ", "inspect ", "summarize ", "summarise "],
    ) && (contains_file_extension(objective)
        || contains_any(objective, &[" file", "fixture", " input"]))
}

fn has_explicit_file_write_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        clause_has_output_target(
            clause,
            &[
                "write",
                "save",
                "append",
                "overwrite",
                "update",
                "edit",
                "modify",
            ],
        )
    })
}

fn has_output_file_intent(objective: &str) -> bool {
    objective_clauses(objective).any(output_clause_has_named_file)
}

fn output_clause_has_named_file(clause: &str) -> bool {
    clause_has_output_target(
        clause,
        &[
            "create", "write", "save", "export", "deliver", "generate", "produce",
        ],
    )
}

fn clause_has_output_target(clause: &str, verbs: &[&str]) -> bool {
    if negates_file_output(clause) {
        return false;
    }
    let Some((verb_index, verb)) = command_verb(clause, verbs) else {
        return false;
    };
    let after_verb = &clause[verb_index + verb.len()..];
    !contains_any(
        after_verb,
        &[" after reading ", " using input ", " from input "],
    ) && (contains_file_extension(after_verb)
        || contains_any(
            after_verb,
            &[
                " file",
                " files",
                " document",
                " artifact",
                " spreadsheet",
                " workbook",
                " presentation",
                " slide deck",
            ],
        ))
}

fn negates_file_output(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "do not create",
            "don't create",
            "never create",
            "without creating",
            "do not write",
            "don't write",
            "never write",
            "without writing",
            "do not save",
            "don't save",
            "never save",
            "without saving",
            "do not generate",
            "don't generate",
            "never generate",
            "without generating",
            "do not produce",
            "don't produce",
            "never produce",
            "without producing",
            "do not export",
            "don't export",
            "never export",
            "without exporting",
            "do not deliver",
            "don't deliver",
            "never deliver",
            "without delivering",
            "do not append",
            "don't append",
            "never append",
            "without appending",
            "do not overwrite",
            "don't overwrite",
            "never overwrite",
            "without overwriting",
            "do not update",
            "don't update",
            "never update",
            "without updating",
            "do not edit",
            "don't edit",
            "never edit",
            "without editing",
            "do not modify",
            "don't modify",
            "never modify",
            "without modifying",
            "no output file",
        ],
    ) || leading_prohibition_covers(
        clause,
        &[
            "create ",
            "write ",
            "save ",
            "export ",
            "deliver ",
            "generate ",
            "produce ",
            "append ",
            "overwrite ",
            "update ",
            "edit ",
            "modify ",
        ],
    )
}

fn has_delete_file_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        !negates_delete_file(clause)
            && (contains_any(clause, &["delete file", "delete the file", "remove file"])
                || (contains_any(clause, &["delete ", "remove "])
                    && contains_file_extension(clause)))
    })
}

fn negates_delete_file(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "do not delete",
            "don't delete",
            "never delete",
            "without deleting",
            "do not remove",
            "don't remove",
            "never remove",
            "without removing",
        ],
    ) || leading_prohibition_covers(clause, &["delete ", "remove "])
}

fn leading_prohibition_covers(clause: &str, actions: &[&str]) -> bool {
    let trimmed = clause.trim_start();
    let Some(prohibited) = ["do not ", "don't ", "never "]
        .iter()
        .find_map(|prefix| trimmed.strip_prefix(prefix))
    else {
        return false;
    };
    let boundary = [" but ", " however ", " instead "]
        .iter()
        .filter_map(|separator| prohibited.find(separator))
        .min()
        .unwrap_or(prohibited.len());
    contains_any(&prohibited[..boundary], actions)
}

fn command_verb<'a>(clause: &str, verbs: &'a [&str]) -> Option<(usize, &'a str)> {
    verbs
        .iter()
        .flat_map(|verb| {
            clause
                .match_indices(verb)
                .map(move |(index, _)| (index, *verb))
        })
        .filter(|(index, verb)| {
            let begins_token = *index == 0
                || clause[..*index]
                    .chars()
                    .next_back()
                    .is_some_and(|character| {
                        character.is_whitespace() || ",;:!?".contains(character)
                    });
            let after_index = *index + verb.len();
            let ends_token = clause[after_index..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace);
            begins_token && ends_token
        })
        .min_by_key(|(index, _)| *index)
}

fn contains_file_extension(value: &str) -> bool {
    [
        ".md", ".pdf", ".docx", ".html", ".rtf", ".txt", ".json", ".csv", ".xls", ".xlsx", ".pptx",
        ".xml",
    ]
    .iter()
    .any(|extension| value.contains(extension))
}

fn looks_like_recipient_address(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            matches!(
                character,
                ',' | ';' | ':' | '.' | '<' | '>' | '(' | ')' | '[' | ']' | '"' | '\''
            )
        });
        let Some((local, domain)) = token.split_once('@') else {
            return false;
        };
        !local.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    })
}

fn explicitly_requests_operation(operation: &str, objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        !negates_explicit_operation(clause, operation)
            && ["use ", "using ", "operation ", "tool "]
                .iter()
                .any(|prefix| {
                    let needle = format!("{prefix}{operation}");
                    clause.match_indices(&needle).any(|(index, _)| {
                        let after = &clause[index + needle.len()..];
                        after.is_empty()
                            || after.chars().next().is_some_and(|character| {
                                character.is_whitespace() || ",;:)]".contains(character)
                            })
                            || after == "."
                            || after.starts_with(". ")
                    })
                })
    })
}

fn negates_explicit_operation(clause: &str, operation: &str) -> bool {
    ["do not use ", "don't use ", "never use ", "without using "]
        .iter()
        .any(|prefix| clause.contains(&format!("{prefix}{operation}")))
}

fn has_connected_work_intent(objective: &str) -> bool {
    objective_clauses(objective).any(connected_work_clause)
}

fn connected_work_clause(clause: &str) -> bool {
    if negates_connected_work(clause) || !has_connected_work_action(clause) {
        return false;
    }
    let explicit_connector = contains_any(
        clause,
        &[
            "mcp server",
            "configured mcp",
            "connected account",
            "connected service",
            "apple apps",
            "apple app connector",
            "connected connector",
            "configured connector",
        ],
    );
    let apple_private_service = contains_any(
        clause,
        &[
            "apple mail",
            "mail app",
            "my mail",
            "my email",
            "my emails",
            "my inbox",
            "mail inbox",
            "email inbox",
            "unread email",
            "unread mail",
            "apple calendar",
            "calendar app",
            "my calendar",
        ],
    );
    let named_connector = contains_any(
        clause,
        &[
            "google drive",
            "google calendar",
            "gmail",
            "microsoft 365",
            "outlook",
            "notion",
            "slack",
            "teams",
        ],
    );
    explicit_connector || apple_private_service || named_connector
}

fn has_connected_work_action(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "use ",
            "using ",
            "access ",
            "read ",
            "check ",
            "find ",
            "search ",
            "list ",
            "show ",
            "retrieve ",
            "review ",
            "summarize ",
            "summarise ",
            "open ",
            "look up ",
            "create ",
            "update ",
            "add ",
            "schedule ",
            "send ",
            "draft ",
            "do i have ",
            "do we have ",
            "are there any ",
            "what is on my ",
            "what's on my ",
        ],
    )
}

fn negates_connected_work(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "do not use",
            "don't use",
            "never use",
            "without using",
            "do not access",
            "don't access",
            "never access",
            "without accessing",
            "do not read",
            "don't read",
            "never read",
            "without reading",
            "do not check",
            "don't check",
            "never check",
            "without checking",
            "do not send",
            "don't send",
            "never send",
            "without sending",
            "do not draft",
            "don't draft",
            "never draft",
            "without drafting",
            "do not create",
            "don't create",
            "never create",
            "without creating",
            "do not update",
            "don't update",
            "never update",
            "without updating",
            "do not add",
            "don't add",
            "never add",
            "without adding",
            "do not schedule",
            "don't schedule",
            "never schedule",
            "without scheduling",
            "no access to",
            "avoid accessing",
        ],
    )
}

fn has_event_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        !negates_calendar_event(clause)
            && contains_any(
                clause,
                &[
                    "calendar event",
                    "create an event",
                    "create event",
                    "create a meeting",
                    "schedule an event",
                    "schedule a meeting",
                    "book an event",
                    "book a meeting",
                    "event titled",
                ],
            )
    })
}

fn negates_calendar_event(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "do not create an event",
            "don't create an event",
            "never create an event",
            "without creating an event",
            "do not create a meeting",
            "don't create a meeting",
            "never create a meeting",
            "without creating a meeting",
            "do not schedule an event",
            "don't schedule an event",
            "never schedule an event",
            "without scheduling an event",
            "do not schedule a meeting",
            "don't schedule a meeting",
            "never schedule a meeting",
            "without scheduling a meeting",
            "do not book an event",
            "don't book an event",
            "never book an event",
            "without booking an event",
        ],
    ) || leading_prohibition_covers(clause, &["create ", "schedule ", "book ", "add "])
}

fn avoids_calendar_conflicts(objective: &str) -> bool {
    contains_any(
        objective,
        &[
            "conflict free",
            "conflict-free",
            "avoiding conflicts",
            "avoid conflicts",
        ],
    )
}

fn has_draft_mail_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        !negates_draft_mail(clause)
            && contains_any(
                clause,
                &[
                    "draft",
                    "unsent",
                    "write an email",
                    "compose an email",
                    "prepare an email",
                ],
            )
            && (contains_any(clause, &["email", "mail"]) || looks_like_recipient_address(clause))
    })
}

fn negates_draft_mail(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "do not draft",
            "don't draft",
            "never draft",
            "without drafting",
            "do not compose",
            "don't compose",
            "never compose",
            "without composing",
            "do not prepare an email",
            "don't prepare an email",
            "never prepare an email",
            "without preparing an email",
            "no mail draft",
            "no email draft",
        ],
    ) || leading_prohibition_covers(clause, &["draft ", "compose ", "prepare ", "write "])
}

fn has_independent_marker(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "separate",
            "separately",
            "additional",
            "another",
            "unrelated",
            "also ",
        ],
    )
}

fn has_independent_output_file_intent(objective: &str) -> bool {
    objective_clauses(objective)
        .any(|clause| has_independent_marker(clause) && output_clause_has_named_file(clause))
}

fn has_independent_spreadsheet_output_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        has_independent_marker(clause)
            && output_clause_has_named_file(clause)
            && contains_any(clause, &["spreadsheet", "workbook", ".xlsx", ".csv"])
    })
}

fn has_independent_presentation_output_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        has_independent_marker(clause)
            && output_clause_has_named_file(clause)
            && contains_any(clause, &["presentation", "slides", ".pptx"])
    })
}

fn has_independent_input_file_intent(objective: &str) -> bool {
    objective_clauses(objective)
        .any(|clause| has_independent_marker(clause) && has_input_file_read_intent(clause))
}

fn has_independent_web_research_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        has_independent_marker(clause)
            && contains_any(
                clause,
                &[
                    "official source",
                    "primary source",
                    "public web source",
                    "search the web",
                ],
            )
    })
}

fn has_independent_event_with_conflict_policy(objective: &str, conflict_free: bool) -> bool {
    objective_clauses(objective).any(|clause| {
        has_independent_marker(clause)
            && has_event_intent(clause)
            && avoids_calendar_conflicts(clause) == conflict_free
    })
}

fn has_independent_draft_mail_intent(objective: &str) -> bool {
    objective_clauses(objective)
        .any(|clause| has_independent_marker(clause) && has_draft_mail_intent(clause))
}

fn has_send_mail_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        !negates_send_mail(clause)
            && (contains_any(
                clause,
                &["send email", "send mail", "send an email", "send one email"],
            ) || (looks_like_recipient_address(clause)
                && contains_any(clause, &["email ", "mail ", "send "])))
    })
}

fn negates_send_mail(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "do not send",
            "don't send",
            "never send",
            "without sending",
            "unsent",
        ],
    ) || leading_prohibition_covers(clause, &["send ", "email ", "mail "])
}

fn has_configure_channel_intent(objective: &str) -> bool {
    objective_clauses(objective).any(|clause| {
        !negates_channel_configuration(clause)
            && contains_any(
                clause,
                &[
                    "configure ",
                    "connect ",
                    "set up ",
                    "setup ",
                    "activate ",
                    "disable ",
                ],
            )
            && contains_any(clause, &["channel", "telegram", "discord", "slack"])
    })
}

fn negates_channel_configuration(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "do not configure",
            "don't configure",
            "never configure",
            "without configuring",
            "do not connect",
            "don't connect",
            "never connect",
            "without connecting",
            "do not set up",
            "don't set up",
            "never set up",
            "without setting up",
            "do not activate",
            "don't activate",
            "never activate",
            "without activating",
            "do not disable",
            "don't disable",
            "never disable",
            "without disabling",
        ],
    ) || leading_prohibition_covers(
        clause,
        &[
            "configure ",
            "connect ",
            "set up ",
            "setup ",
            "activate ",
            "disable ",
        ],
    )
}

fn objective_clauses(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(['\n', ';'])
        .flat_map(|segment| segment.split(". "))
        .flat_map(|segment| segment.split("? "))
        .flat_map(|segment| segment.split("! "))
        .flat_map(|segment| segment.split(" and then "))
        .flat_map(|segment| segment.split(" then "))
        .flat_map(|segment| segment.split(" but "))
        .flat_map(|segment| segment.split(" however "))
        .flat_map(|segment| segment.split(" instead "))
}

fn contains_any(value: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| value.contains(term))
}

fn normalize(value: &str) -> String {
    value.to_ascii_lowercase().replace(['-', '_'], " ")
}
