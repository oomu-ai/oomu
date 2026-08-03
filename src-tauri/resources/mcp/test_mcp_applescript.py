import hashlib
import json
import os
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import mcp_applescript as mcp


def structured_error(error_type="timeout", message="AppleScript execution timed out after 20s."):
    return json.dumps(
        {
            "status": "error",
            "error_type": error_type,
            "message": message,
        }
    )


def settled_draft(draft_id="draft-42"):
    return patch.object(
        mcp,
        "wait_for_exact_mail_draft",
        return_value={"draftId": draft_id, "settled": True},
    )


def unsettled_draft(result=None):
    return patch.object(
        mcp,
        "wait_for_exact_mail_draft",
        return_value={} if result is None else result,
    )


def clean_mail_baseline():
    return patch.object(mcp, "inspect_exact_mail_draft", return_value={})


def prepared_draft(draft_id="saved-prepared-42"):
    return patch.object(
        mcp,
        "wait_for_mail_draft_subject_token",
        return_value={"draftId": draft_id, "settled": True},
    )


def settled_token_absence():
    return patch.object(
        mcp,
        "wait_for_mail_draft_subject_token_absence",
        return_value={"absent": True, "settled": True},
    )


class FakeProcess:
    def __init__(self, stdout="", stderr="", returncode=0, timeout_on_first=False, timeout_on_second=False):
        self.stdout = stdout
        self.stderr = stderr
        self.returncode = returncode
        self.timeout_on_first = timeout_on_first
        self.timeout_on_second = timeout_on_second
        self.communicate_calls = 0
        self.terminated = False
        self.killed = False

    def communicate(self, timeout=None):
        self.communicate_calls += 1
        if self.communicate_calls == 1 and self.timeout_on_first:
            raise subprocess.TimeoutExpired(cmd=["osascript"], timeout=timeout)
        if self.communicate_calls == 2 and self.timeout_on_second:
            raise subprocess.TimeoutExpired(cmd=["osascript"], timeout=timeout)
        return self.stdout, self.stderr

    def terminate(self):
        self.terminated = True

    def kill(self):
        self.killed = True


class AppleScriptTimeoutHandlingTests(unittest.TestCase):
    def test_collection_readers_publish_empty_success_contracts(self):
        tools = {tool["name"]: tool for tool in mcp.tool_list()}

        for tool_name, collection_name in {
            "read_system_calendar": "events",
            "read_system_emails": "emails",
            "read_system_notes": "notes",
            "read_system_contacts": "contacts",
            "read_system_music": "songs",
            "read_system_photos": "photos",
            "read_system_reminders": "reminders",
            "read_apple_app_ui": "uiText",
        }.items():
            contract = tools[tool_name]["outputSchema"]["x-oomu-result-contract"]
            self.assertEqual(contract["kind"], "collection")
            self.assertEqual(contract["path"], f"/structuredContent/{collection_name}")
            self.assertTrue(contract["emptyIsSuccess"])

    def test_native_photo_and_music_tools_never_simulate_helper_results(self):
        for tool_name in ["read_system_music", "read_system_photos"]:
            result = mcp.call_tool({"name": tool_name, "arguments": {}})
            self.assertTrue(result["isError"])
            self.assertEqual(
                result["structuredContent"]["code"],
                "native_read_boundary_required",
            )

    def test_notification_helper_never_claims_unverified_delivery(self):
        result = mcp.trigger_system_notification({"body_text": "Ready"})
        self.assertTrue(result["isError"])
        self.assertEqual(
            result["structuredContent"]["code"],
            "notification_native_boundary_required",
        )
        self.assertFalse(result["structuredContent"]["verified"])

    def test_run_applescript_timeout_returns_structured_error(self):
        process = FakeProcess(timeout_on_first=True, timeout_on_second=True)
        with patch.object(
            mcp.subprocess,
            "Popen",
            return_value=process,
        ) as popen:
            output = mcp.run_applescript("return 1", timeout=2)

        payload = json.loads(output)
        self.assertEqual(payload["status"], "error")
        self.assertEqual(payload["error_type"], "timeout")
        self.assertEqual(payload["message"], "AppleScript execution timed out after 2s.")
        self.assertEqual(process.communicate_calls, 3)
        self.assertTrue(process.terminated)
        self.assertTrue(process.killed)
        self.assertEqual(popen.call_args.args[0], [mcp.OSASCRIPT_PATH, "-e", "return 1"])

    def test_run_applescript_failure_returns_structured_error(self):
        process = FakeProcess(stderr="Calendar is not authorized", returncode=1)
        with patch.object(
            mcp.subprocess,
            "Popen",
            return_value=process,
        ):
            output = mcp.run_applescript("return 1")

        payload = json.loads(output)
        self.assertEqual(payload["status"], "error")
        self.assertEqual(payload["error_type"], "execution_failed")
        self.assertEqual(payload["message"], "Calendar is not authorized")

    def test_applescript_timeouts_default_to_twenty_seconds(self):
        self.assertEqual(mcp.DEFAULT_TIMEOUT_SECONDS, 20)
        self.assertEqual(mcp.MAIL_READ_TIMEOUT_SECONDS, 20)
        self.assertEqual(mcp.UI_AUTOMATION_TIMEOUT_SECONDS, 20)

    def test_mail_internal_replay_and_postcondition_flags_are_not_advertised(self):
        tool = next(tool for tool in mcp.tool_list() if tool["name"] == "draft_system_email")
        properties = tool["inputSchema"]["properties"]
        self.assertNotIn("reuse_existing_matching", properties)
        self.assertNotIn("verify_existing_only", properties)

    def test_message_prepare_tool_is_bounded_and_never_accepts_send_controls(self):
        tool = next(tool for tool in mcp.tool_list() if tool["name"] == "prepare_system_message")
        properties = tool["inputSchema"]["properties"]
        self.assertEqual(set(properties), {"recipient", "body"})
        self.assertEqual(tool["inputSchema"]["required"], ["recipient", "body"])
        self.assertFalse(tool["inputSchema"]["additionalProperties"])

    def test_camera_preview_is_declared_but_never_simulated_by_the_helper(self):
        tool = next(tool for tool in mcp.tool_list() if tool["name"] == "preview_camera")
        self.assertEqual(tool["inputSchema"]["properties"], {})
        result = mcp.call_tool({"name": "preview_camera", "arguments": {}})
        self.assertTrue(result["isError"])
        self.assertEqual(
            result["structuredContent"]["code"],
            "camera_preview_native_boundary_required",
        )
        self.assertFalse(result["structuredContent"]["verified"])

    def test_prepare_system_message_verifies_open_composer_without_sending(self):
        output = mcp.FIELD_SEPARATOR.join(["true", "true", "true"])
        with patch.object(mcp, "run_applescript", return_value=output) as run:
            result = mcp.prepare_system_message(
                {"recipient": "test@example.com", "body": "Please review this."}
            )

        self.assertFalse(result["isError"])
        self.assertTrue(result["structuredContent"]["verified"])
        self.assertFalse(result["structuredContent"]["sent"])
        self.assertEqual(result["structuredContent"]["status"], "prepared")
        script = run.call_args.args[0]
        self.assertIn('tell application "Messages" to activate', script)
        self.assertNotIn("click button", script.lower())
        self.assertNotIn(" keystroke return", script.lower())

    def test_prepare_system_message_rejects_unverified_composer(self):
        output = mcp.FIELD_SEPARATOR.join(["true", "false", "false"])
        with patch.object(mcp, "run_applescript", return_value=output):
            result = mcp.prepare_system_message(
                {"recipient": "test@example.com", "body": "Please review this."}
            )

        self.assertTrue(result["isError"])
        self.assertFalse(result["structuredContent"]["verified"])
        self.assertFalse(result["structuredContent"]["sent"])
        self.assertEqual(
            result["structuredContent"]["code"],
            "message_prepare_verification_failed",
        )

    def test_mail_draft_preflight_returns_typed_nonmutating_failures(self):
        cases = [
            (
                mcp.applescript_error_payload(
                    "execution_failed",
                    "Not authorized to send Apple events to Mail. (-1743)",
                ),
                "mail_automation_permission_required",
            ),
            (structured_error(), "mail_automation_timeout"),
            (
                structured_error("execution_failed", "Mail is not running."),
                "mail_automation_unavailable",
            ),
        ]
        for preflight_output, expected_code in cases:
            with self.subTest(code=expected_code), patch.object(
                mcp, "run_applescript", return_value=preflight_output
            ) as run:
                result = mcp.draft_system_email(
                    {"subject": "Supplier Decision", "body": "Ready."}
                )

            self.assertTrue(result["isError"])
            self.assertEqual(result["structuredContent"]["code"], expected_code)
            self.assertEqual(result["structuredContent"]["failurePhase"], "preflight")
            self.assertEqual(result["structuredContent"]["cleanupState"], "not_required")
            self.assertTrue(result["structuredContent"]["cleanupVerified"])
            self.assertFalse(result["structuredContent"]["residualDraftPossible"])
            self.assertEqual(run.call_count, 1)

    def test_mail_preflight_error_number_is_parsed_without_exposing_raw_error(self):
        output = mcp.applescript_error_payload(
            "execution_failed", "automation denied (-1743)"
        )
        self.assertEqual(json.loads(output)["error_number"], -1743)
        result = mcp.mail_automation_error_result(json.loads(output))
        self.assertNotIn("-1743", result["content"][0]["text"])

    def test_read_system_calendar_fails_when_preflight_times_out(self):
        with patch.object(mcp, "run_applescript", return_value=structured_error()) as run:
            result = mcp.read_system_calendar(
                {
                    "start_date": "2026-07-06T09:00:00",
                    "end_date": "2026-07-06T10:00:00",
                }
            )

        payload = json.loads(result["content"][0]["text"])
        self.assertTrue(result["isError"])
        self.assertEqual(payload, mcp.PERMISSION_BLOCKED_OR_TIMED_OUT_PAYLOAD)
        self.assertEqual(result["structuredContent"], mcp.PERMISSION_BLOCKED_OR_TIMED_OUT_PAYLOAD)
        self.assertEqual(run.call_count, 1)

    def test_read_system_calendar_reports_timeout_as_tool_error(self):
        with patch.object(mcp, "run_applescript", side_effect=["ok", structured_error()]):
            result = mcp.read_system_calendar(
                {
                    "start_date": "2026-07-06T09:00:00",
                    "end_date": "2026-07-06T10:00:00",
                }
            )

        payload = json.loads(result["content"][0]["text"])
        self.assertTrue(result["isError"])
        self.assertEqual(payload["events"], [])
        self.assertEqual(result["structuredContent"]["events"], [])
        self.assertEqual(result["structuredContent"]["warning"], "timeout")

    def test_read_system_reminders_reports_timeout_as_tool_error(self):
        with patch.object(mcp, "run_applescript", side_effect=["ok", structured_error()]):
            result = mcp.read_system_reminders({"list_name": "Reminders"})

        payload = json.loads(result["content"][0]["text"])
        self.assertTrue(result["isError"])
        self.assertEqual(payload["reminders"], [])
        self.assertEqual(result["structuredContent"]["reminders"], [])
        self.assertEqual(result["structuredContent"]["warning"], "timeout")

    def test_read_system_emails_uses_mail_timeout_for_error_results(self):
        with patch.object(mcp, "run_applescript", side_effect=["ok", structured_error()]) as run:
            result = mcp.read_system_emails({"max_messages": 7})

        payload = json.loads(result["content"][0]["text"])
        self.assertTrue(result["isError"])
        self.assertEqual(payload["emails"], [])
        self.assertEqual(result["structuredContent"]["emails"], [])
        self.assertEqual(result["structuredContent"]["warning"], "timeout")
        self.assertEqual(run.call_args.kwargs["timeout"], mcp.MAIL_READ_TIMEOUT_SECONDS)

    def test_read_system_emails_bounds_recent_mail_without_materializing_entire_inbox(self):
        with patch.object(mcp, "run_applescript", side_effect=["ok", ""]) as run:
            result = mcp.read_system_emails({"max_messages": 7})

        script = run.call_args.args[0]
        self.assertFalse(result["isError"])
        self.assertIn("set message_count to count of messages of inbox", script)
        self.assertIn("set message_item to message index of inbox", script)
        self.assertNotIn("set inbox_messages to every message of inbox", script)
        self.assertEqual(run.call_args.kwargs["timeout"], mcp.MAIL_READ_TIMEOUT_SECONDS)

    def test_read_system_emails_keeps_filtered_unread_mail_under_mail_timeout(self):
        with patch.object(mcp, "run_applescript", side_effect=["ok", ""]) as run:
            result = mcp.read_system_emails({"max_messages": 7, "unread_only": True})

        script = run.call_args.args[0]
        self.assertFalse(result["isError"])
        self.assertIn("set inbox_messages to every message of inbox whose read status is false", script)
        self.assertEqual(run.call_args.kwargs["timeout"], mcp.MAIL_READ_TIMEOUT_SECONDS)

    def test_read_system_notes_reports_timeout_as_tool_error(self):
        with patch.object(mcp, "run_applescript", return_value=structured_error()):
            result = mcp.read_system_notes({"max_notes": 7})

        payload = json.loads(result["content"][0]["text"])
        self.assertTrue(result["isError"])
        self.assertEqual(payload["notes"], [])
        self.assertEqual(result["structuredContent"]["notes"], [])
        self.assertEqual(result["structuredContent"]["warning"], "timeout")

    def test_read_apple_app_ui_uses_ui_timeout_and_reports_failure(self):
        with patch.object(mcp, "run_applescript", return_value=structured_error()) as run:
            result = mcp.read_apple_app_ui({"app_name": "Calendar"})

        self.assertEqual(run.call_args.kwargs["timeout"], mcp.UI_AUTOMATION_TIMEOUT_SECONDS)
        self.assertTrue(result["isError"])
        self.assertEqual(result["structuredContent"]["uiText"], [])
        self.assertEqual(result["structuredContent"]["warning"], "timeout")

    def test_notification_helper_requires_native_delivery_verification(self):
        with patch.object(mcp, "run_applescript") as run:
            result = mcp.trigger_system_notification({"body_text": "hello"})
        run.assert_not_called()
        self.assertTrue(result["isError"])
        self.assertEqual(
            result["structuredContent"]["code"],
            "notification_native_boundary_required",
        )

    def test_mail_success_boolean_requires_preflight_and_draft_success(self):
        removed = mcp.FIELD_SEPARATOR.join(["0", "0", "0", "0", "0", "0"])
        with patch.object(
            mcp,
            "run_applescript",
            side_effect=["ok", structured_error(), removed],
        ), clean_mail_baseline():
            failed = mcp.draft_system_email({"subject": "Subject", "body": "Body"})
        self.assertTrue(failed["isError"])
        self.assertTrue(failed["structuredContent"]["cleanupVerified"])
        self.assertEqual(failed["structuredContent"]["cleanupState"], "absent")

        saved = mcp.FIELD_SEPARATOR.join(
            [
                "Subject",
                "Body",
                "",
                "",
                "",
                "outgoing message",
            ]
        )
        with patch.object(
            mcp, "run_applescript", side_effect=["ok", "ok", saved]
        ) as run, clean_mail_baseline(), prepared_draft(), settled_draft(
            "saved-final-42"
        ), settled_token_absence():
            succeeded = mcp.draft_system_email({"subject": "Subject", "body": "Body"})
        self.assertFalse(succeeded["isError"])
        self.assertTrue(succeeded["structuredContent"]["success"])
        self.assertTrue(succeeded["structuredContent"]["saved"])
        self.assertTrue(succeeded["structuredContent"]["verified"])
        self.assertEqual(succeeded["structuredContent"]["draftId"], "saved-final-42")
        self.assertEqual(succeeded["structuredContent"]["draftState"], "outgoing_message")
        self.assertFalse(succeeded["structuredContent"]["sent"])
        self.assertEqual(succeeded["structuredContent"]["exactMatchCount"], 1)
        self.assertTrue(succeeded["structuredContent"]["uniquenessVerified"])
        self.assertEqual(
            succeeded["structuredContent"]["bodySha256"],
            hashlib.sha256(b"Body").hexdigest(),
        )
        self.assertIn("save new_message", run.call_args_list[2].args[0])
        self.assertIn("whose subject is target_subject", run.call_args_list[2].args[0])

    def test_mail_replay_protection_reuses_one_verified_matching_draft(self):
        with patch.object(
            mcp, "run_applescript", side_effect=["ok", "existing-draft-42"]
        ) as run, settled_draft("existing-draft-42"):
            result = mcp.draft_system_email(
                {
                    "to": "owner@example.com",
                    "subject": "Supplier Decision Review",
                    "body": "Decision pack ready.",
                    "reuse_existing_matching": True,
                }
            )

        self.assertFalse(result["isError"])
        self.assertEqual(result["structuredContent"]["draftId"], "existing-draft-42")
        self.assertTrue(result["structuredContent"]["reusedExisting"])
        self.assertEqual(result["structuredContent"]["exactMatchCount"], 1)
        self.assertTrue(result["structuredContent"]["uniquenessVerified"])
        self.assertEqual(run.call_count, 2)
        search_script = run.call_args_list[1].args[0]
        self.assertEqual(
            search_script.count(
                "repeat with candidate_message in (every message of drafts mailbox whose subject is expected_subject)"
            ),
            1,
        )
        self.assertEqual(
            search_script.count(
                "repeat with candidate_message in (every message of sent mailbox whose subject is expected_subject)"
            ),
            1,
        )
        self.assertIn("set expected_body_variants to", search_script)
        self.assertNotIn("make new outgoing message", search_script)

    def test_send_mail_sends_once_and_verifies_sent_mail(self):
        with patch.object(
            mcp, "run_applescript", side_effect=["ok", "ok"]
        ) as run, patch.object(
            mcp, "inspect_exact_sent_email", return_value={}
        ), patch.object(
            mcp,
            "wait_for_exact_sent_email",
            return_value={"sentMessageId": "sent-42", "settled": True},
        ):
            result = mcp.send_system_email(
                {
                    "to": "owner@example.com",
                    "subject": "OOMU Test — Supplier Exception",
                    "body": "Report: supplier_exception_2026-07-21_10-30.md",
                }
            )

        self.assertFalse(result["isError"])
        self.assertTrue(result["structuredContent"]["sent"])
        self.assertTrue(result["structuredContent"]["verified"])
        self.assertEqual(result["structuredContent"]["exactMatchCount"], 1)
        self.assertTrue(result["structuredContent"]["uniquenessVerified"])
        self.assertFalse(result["structuredContent"]["reusedExisting"])
        send_script = run.call_args_list[1].args[0]
        self.assertIn("send new_message", send_script)
        self.assertNotIn("save new_message", send_script)

    def test_send_mail_reuses_one_exact_sent_message_without_sending_again(self):
        with patch.object(mcp, "run_applescript", return_value="ok") as run, patch.object(
            mcp,
            "inspect_exact_sent_email",
            return_value={"sentMessageId": "sent-existing-42"},
        ):
            result = mcp.send_system_email(
                {
                    "to": "owner@example.com",
                    "subject": "OOMU Test — Supplier Exception",
                    "body": "Exact body",
                }
            )

        self.assertFalse(result["isError"])
        self.assertTrue(result["structuredContent"]["reusedExisting"])
        self.assertEqual(result["structuredContent"]["sentMessageId"], "sent-existing-42")
        self.assertEqual(run.call_count, 1)

    def test_send_mail_attaches_and_verifies_the_exact_local_report(self):
        with tempfile.TemporaryDirectory() as root:
            attachment_path = os.path.join(root, "verified-report.md")
            with open(attachment_path, "wb") as output:
                output.write(b"verified report")
            with patch.object(
                mcp, "run_applescript", side_effect=["ok", "ok"]
            ) as run, patch.object(
                mcp, "inspect_exact_sent_email", return_value={}
            ), patch.object(
                mcp,
                "wait_for_exact_sent_email",
                return_value={"sentMessageId": "sent-attachment", "settled": True},
            ):
                result = mcp.send_system_email(
                    {
                        "to": "owner@example.com",
                        "subject": "Verified report",
                        "body": "The verified report is attached.",
                        "attachmentPath": attachment_path,
                    }
                )

        self.assertFalse(result["isError"])
        self.assertEqual(
            result["structuredContent"]["attachmentName"], "verified-report.md"
        )
        self.assertEqual(
            result["structuredContent"]["attachmentSha256"],
            hashlib.sha256(b"verified report").hexdigest(),
        )
        self.assertTrue(result["structuredContent"]["attachmentVerified"])
        send_script = run.call_args_list[1].args[0]
        self.assertIn("make new attachment", send_script)
        self.assertIn(attachment_path, send_script)

    def test_send_mail_unverified_postcondition_forbids_blind_retry(self):
        with patch.object(
            mcp, "run_applescript", side_effect=["ok", "ok"]
        ), patch.object(
            mcp, "inspect_exact_sent_email", return_value={}
        ), patch.object(mcp, "wait_for_exact_sent_email", return_value={}):
            result = mcp.send_system_email(
                {
                    "to": "owner@example.com",
                    "subject": "OOMU Test — Supplier Exception",
                    "body": "Exact body",
                }
            )

        self.assertTrue(result["isError"])
        self.assertEqual(
            result["structuredContent"]["code"], "mail_send_result_unverified"
        )
        self.assertEqual(result["structuredContent"]["changedState"], "unverified")
        self.assertIn("Review Sent Mail before retrying", result["content"][0]["text"])

    def test_mail_replay_protection_refuses_duplicate_existing_drafts(self):
        with patch.object(
            mcp,
            "run_applescript",
            side_effect=["ok", "OOMU_MULTIPLE_MATCHING_DRAFTS"],
        ) as run:
            result = mcp.draft_system_email(
                {
                    "subject": "Supplier Decision Review",
                    "body": "Decision pack ready.",
                    "reuse_existing_matching": True,
                }
            )

        self.assertTrue(result["isError"])
        self.assertTrue(result["structuredContent"]["residualDraftPossible"])
        self.assertIn(
            "multiple unsent drafts with this subject and To recipient list",
            result["content"][0]["text"],
        )
        self.assertEqual(run.call_count, 2)

    def test_mail_reused_draft_must_remain_the_only_exact_match_after_save(self):
        with patch.object(
            mcp,
            "run_applescript",
            side_effect=["ok", "existing-draft-42"],
        ), unsettled_draft(
            {
                "error": "Mail contains multiple identical unsent drafts.",
                "residual": True,
                "duplicates": True,
            }
        ):
            result = mcp.draft_system_email(
                {
                    "subject": "Supplier Decision Review",
                    "body": "Decision pack ready.",
                    "reuse_existing_matching": True,
                }
            )

        self.assertTrue(result["isError"])
        self.assertFalse(result["structuredContent"]["verified"])
        self.assertTrue(result["structuredContent"]["residualDraftPossible"])
        self.assertEqual(
            result["structuredContent"]["code"], "mail_draft_review_required"
        )

    def test_mail_postcondition_inventory_is_strictly_nonmutating(self):
        with patch.object(
            mcp, "run_applescript", side_effect=["ok", "existing-draft-42"]
        ) as run:
            result = mcp.draft_system_email(
                {
                    "to": "owner@example.com",
                    "subject": "Supplier Decision Review",
                    "body": "Decision pack ready.",
                    "verify_existing_only": True,
                }
            )

        self.assertFalse(result["isError"])
        self.assertTrue(result["structuredContent"]["postconditionOnly"])
        self.assertEqual(result["structuredContent"]["exactMatchCount"], 1)
        inventory_script = run.call_args_list[1].args[0]
        self.assertIn("every message of drafts mailbox", inventory_script)
        self.assertNotIn("every outgoing message", inventory_script)
        for mutation in [
            "make new outgoing message",
            "save ",
            "set visible",
            "activate",
            "delete ",
        ]:
            self.assertNotIn(mutation, inventory_script)

    def test_mail_postcondition_inventory_rejects_zero_or_duplicate_matches(self):
        for inventory, residual in [
            ("", False),
            ("OOMU_MULTIPLE_MATCHING_DRAFTS", True),
        ]:
            with self.subTest(inventory=inventory), patch.object(
                mcp, "run_applescript", side_effect=["ok", inventory]
            ):
                result = mcp.draft_system_email(
                    {
                        "subject": "Supplier Decision Review",
                        "body": "Decision pack ready.",
                        "verify_existing_only": True,
                    }
                )
            self.assertTrue(result["isError"])
            self.assertFalse(result["structuredContent"]["verified"])
            self.assertEqual(
                result["structuredContent"]["residualDraftPossible"], residual
            )

    def test_mail_replay_protection_flag_must_be_boolean(self):
        with self.assertRaises(mcp.ToolInputError):
            mcp.draft_system_email(
                {
                    "subject": "Supplier Decision Review",
                    "body": "Decision pack ready.",
                    "reuse_existing_matching": "yes",
                }
            )

    def test_mail_draft_fails_when_saved_readback_does_not_match(self):
        mismatched = mcp.FIELD_SEPARATOR.join(
            [
                "Different subject",
                "Body",
                "owner@example.com",
                "",
                "",
                "outgoing message",
            ]
        )
        with patch.object(
            mcp, "run_applescript", side_effect=["ok", "ok", mismatched]
        ), clean_mail_baseline(), prepared_draft(), unsettled_draft(), patch.object(
            mcp,
            "remove_mail_draft_by_id",
            return_value=mcp.mail_cleanup_result("absent"),
        ):
            result = mcp.draft_system_email(
                {
                    "to": "owner@example.com",
                    "subject": "Subject",
                    "body": "Body",
                }
            )

        self.assertTrue(result["isError"])
        self.assertFalse(result["structuredContent"]["verified"])
        self.assertTrue(result["structuredContent"]["cleanupVerified"])
        self.assertFalse(result["structuredContent"]["residualDraftPossible"])
        self.assertEqual(result["structuredContent"]["cleanupState"], "absent")
        self.assertEqual(
            result["structuredContent"]["code"],
            "mail_draft_creation_failed_cleanly",
        )
        self.assertEqual(
            result["structuredContent"]["failurePhase"], "populate_verify"
        )

    def test_mail_post_save_error_removes_the_known_bootstrap_draft(self):
        with patch.object(
            mcp,
            "run_applescript",
            side_effect=["ok", "ok", structured_error()],
        ), clean_mail_baseline(), prepared_draft(), unsettled_draft(), patch.object(
            mcp,
            "remove_mail_draft_by_id",
            return_value=mcp.mail_cleanup_result("absent"),
        ):
            result = mcp.draft_system_email({"subject": "Subject", "body": "Body"})

        self.assertTrue(result["isError"])
        self.assertTrue(result["structuredContent"]["cleanupVerified"])
        self.assertFalse(result["structuredContent"]["residualDraftPossible"])
        self.assertEqual(result["structuredContent"]["cleanupState"], "absent")

    def test_mail_duplicate_after_creation_compensates_the_new_draft(self):
        duplicate_readback = mcp.FIELD_SEPARATOR.join(
            [
                "Subject",
                "Body",
                "",
                "",
                "",
                "outgoing message",
            ]
        )
        with patch.object(
            mcp,
            "run_applescript",
            side_effect=["ok", "ok", duplicate_readback],
        ) as run, unsettled_draft(
            {
                "error": "Mail contains multiple identical unsent drafts.",
                "residual": True,
                "duplicates": True,
            }
        ), clean_mail_baseline(), prepared_draft(), patch.object(
            mcp,
            "remove_mail_draft_by_id",
            return_value=mcp.mail_cleanup_result("absent"),
        ) as cleanup:
            result = mcp.draft_system_email({"subject": "Subject", "body": "Body"})

        self.assertTrue(result["isError"])
        self.assertFalse(result["structuredContent"]["verified"])
        self.assertTrue(result["structuredContent"]["cleanupVerified"])
        self.assertTrue(result["structuredContent"]["residualDraftPossible"])
        self.assertEqual(
            result["structuredContent"]["code"], "mail_draft_review_required"
        )
        self.assertEqual(result["structuredContent"]["failurePhase"], "postcondition")
        self.assertIn("exactly one matching unsent draft", result["content"][0]["text"])
        cleanup.assert_called_once()

    def test_mail_normalizes_crlf_before_native_write_and_receipt_hashing(self):
        saved = mcp.FIELD_SEPARATOR.join(
            [
                "Subject",
                "Line one\nLine two ",
                "",
                "",
                "",
                "outgoing message",
            ]
        )
        with patch.object(
            mcp, "run_applescript", side_effect=["ok", "ok", saved]
        ), clean_mail_baseline(), prepared_draft(), settled_draft(
            "saved-final-42"
        ), settled_token_absence():
            result = mcp.draft_system_email(
                {"subject": "Subject", "body": "Line one\r\nLine two"}
            )

        self.assertFalse(result["isError"])
        self.assertEqual(
            result["structuredContent"]["bodySha256"],
            hashlib.sha256(b"Line one\nLine two").hexdigest(),
        )

    def test_mail_readback_preserves_an_input_trailing_space(self):
        saved = mcp.FIELD_SEPARATOR.join(
            [
                "Subject",
                "Body  ",
                "",
                "",
                "",
                "outgoing message",
            ]
        )
        with patch.object(
            mcp, "run_applescript", side_effect=["ok", "ok", saved]
        ), clean_mail_baseline(), prepared_draft(), settled_draft(
            "saved-final-42"
        ), settled_token_absence():
            result = mcp.draft_system_email(
                {"subject": "Subject", "body": "Body "}
            )

        self.assertFalse(result["isError"])
        self.assertEqual(
            result["structuredContent"]["bodySha256"],
            hashlib.sha256(b"Body ").hexdigest(),
        )

    def test_mail_readback_rejects_more_than_one_added_trailing_space(self):
        saved = mcp.FIELD_SEPARATOR.join(
            ["Subject", "Body  ", "", "", "", "outgoing message"]
        )
        with patch.object(
            mcp,
            "run_applescript",
            side_effect=["ok", "ok", saved],
        ), clean_mail_baseline(), prepared_draft(), unsettled_draft(), patch.object(
            mcp,
            "remove_mail_draft_by_id",
            return_value=mcp.mail_cleanup_result("absent"),
        ):
            result = mcp.draft_system_email(
                {"subject": "Subject", "body": "Body"}
            )

        self.assertTrue(result["isError"])
        self.assertFalse(result["structuredContent"]["verified"])

    def test_mail_mismatched_readback_id_cleans_up_the_bootstrap_identity(self):
        mismatched_id = mcp.FIELD_SEPARATOR.join(
            ["Different", "Body ", "", "", "", "outgoing message"]
        )
        with patch.object(
            mcp,
            "run_applescript",
            side_effect=["ok", "ok", mismatched_id],
        ), clean_mail_baseline(), prepared_draft("bootstrap-id"), unsettled_draft(), patch.object(
            mcp,
            "remove_mail_draft_by_id",
            return_value=mcp.mail_cleanup_result("absent"),
        ) as cleanup:
            result = mcp.draft_system_email(
                {"subject": "Subject", "body": "Body"}
            )

        self.assertTrue(result["isError"])
        self.assertEqual(cleanup.call_args.args[0], "bootstrap-id")

    def test_mail_readback_does_not_trim_a_requested_trailing_space(self):
        self.assertFalse(mcp.mail_body_readback_matches("Body ", "Body"))
        self.assertTrue(mcp.mail_body_readback_matches("Body ", "Body  "))

    def test_mail_readback_accepts_only_enumerated_native_paragraph_variants(self):
        requested = "Section one\r\n\r\nSection two"
        self.assertTrue(mcp.mail_body_readback_matches("", " \n"))
        self.assertFalse(mcp.mail_body_readback_matches("", "  \n"))
        self.assertTrue(
            mcp.mail_body_readback_matches(
                requested, "Section one\n\u2028\nSection two \n"
            )
        )
        self.assertTrue(
            mcp.mail_body_readback_matches(
                requested, "Section one\n\u2029\nSection two "
            )
        )
        self.assertTrue(
            mcp.mail_body_readback_matches(
                requested, "\nSection one\n\nSection two \n"
            )
        )
        self.assertFalse(
            mcp.mail_body_readback_matches(
                requested, "\n\nSection one\n\nSection two \n"
            )
        )
        self.assertFalse(
            mcp.mail_body_readback_matches(
                requested, "Section one\u2028Section two \n"
            )
        )
        self.assertFalse(
            mcp.mail_body_readback_matches(
                requested, "Section one\n\u2028\nSection changed \n"
            )
        )
        self.assertFalse(
            mcp.mail_body_readback_matches(
                requested, "Section one\n\u2028\nSection two  \n"
            )
        )

    def test_mail_native_body_variants_are_bounded_and_preserve_requested_content(self):
        variants = mcp.mail_body_readback_variants("A\n\n\nB")
        self.assertLessEqual(len(variants), 18)
        self.assertIn("A\n\u2028\n\u2028\nB \n", variants)
        self.assertIn("A\n\u2029\n\u2029\nB", variants)
        self.assertIn("\nA\n\n\nB \n", variants)
        self.assertNotIn("A\n\nB", variants)

    def test_mail_exact_inventory_uses_native_variants_and_dereferenced_recipients(self):
        with patch.object(mcp, "run_applescript", return_value="") as run:
            mcp.inspect_exact_mail_draft(
                ["owner@example.com"],
                [],
                [],
                "Subject",
                "First\n\nSecond",
            )

        script = run.call_args.args[0]
        self.assertIn("set expected_body_variants to", script)
        self.assertIn("\u2028", script)
        self.assertIn("\u2029", script)
        self.assertEqual(script.count('tell application "Mail"'), 2)
        self.assertIn("set recipient_value to contents of recipient_ref", script)
        self.assertIn("(get address of recipient_value) as string", script)
        self.assertNotIn("address of recipient_ref as string", script)

    def test_mail_authoritative_inventory_prohibits_altered_body_sent_collision(self):
        with patch.object(
            mcp, "run_applescript", return_value="OOMU_MATCHING_SENT_MESSAGE"
        ) as run:
            result = mcp.inspect_exact_mail_draft(
                ["owner@example.com"], [], [], "Subject", "Body"
            )

        self.assertTrue(result["sent"])
        self.assertTrue(result["residual"])
        self.assertIn("subject and To recipient list", result["error"])
        script = run.call_args.args[0]
        self.assertIn("every message of drafts mailbox", script)
        self.assertIn("every message of sent mailbox", script)
        sent_inventory = script[
            script.index("set matching_sent to {}") : script.index(
                "set exact_drafts to {}"
            )
        ]
        self.assertIn(
            "subject of candidate_message as string) is expected_subject",
            sent_inventory,
        )
        self.assertIn(
            "recipient_addresses(to recipients of candidate_message) is expected_to",
            sent_inventory,
        )
        self.assertNotIn("candidate_body", sent_inventory)
        self.assertNotIn("expected_cc", sent_inventory)
        self.assertNotIn("expected_bcc", sent_inventory)
        self.assertLess(
            script.index("count of matching_sent"),
            script.index("count of matching_drafts"),
        )

    def test_mail_authoritative_inventory_treats_altered_body_draft_as_collision(self):
        with patch.object(
            mcp, "run_applescript", return_value="OOMU_MATCHING_DRAFT_COLLISION"
        ) as run:
            result = mcp.inspect_exact_mail_draft(
                ["owner@example.com"], [], [], "Subject", "Expected body"
            )

        self.assertTrue(result["collision"])
        self.assertTrue(result["residual"])
        self.assertIn("content or other recipients differ", result["error"])
        script = run.call_args.args[0]
        draft_inventory = script[
            script.index("set matching_drafts to {}") : script.index(
                "set matching_sent to {}"
            )
        ]
        self.assertIn(
            "recipient_addresses(to recipients of candidate_message) is expected_to",
            draft_inventory,
        )
        self.assertNotIn("candidate_body", draft_inventory)
        self.assertNotIn("expected_cc", draft_inventory)
        self.assertNotIn("expected_bcc", draft_inventory)
        self.assertIn(
            "if (count of exact_drafts) is not 1 then return \"OOMU_MATCHING_DRAFT_COLLISION\"",
            script,
        )

    def test_mail_authoritative_inventory_accepts_one_draft_only_after_full_exact_match(self):
        with patch.object(mcp, "run_applescript", return_value="draft-42") as run:
            result = mcp.inspect_exact_mail_draft(
                ["owner@example.com"],
                ["copy@example.com"],
                ["audit@example.com"],
                "Subject",
                "Expected body",
            )

        self.assertEqual(result, {"draftId": "draft-42"})
        script = run.call_args.args[0]
        exact_inventory = script[
            script.index("set exact_drafts to {}") : script.index(
                "if (count of matching_sent)"
            )
        ]
        self.assertIn("set candidate_body", exact_inventory)
        self.assertIn("body_matches", exact_inventory)
        self.assertIn(
            "recipient_addresses(to recipients of candidate_message) is expected_to",
            exact_inventory,
        )
        self.assertIn(
            "recipient_addresses(cc recipients of candidate_message) is expected_cc",
            exact_inventory,
        )
        self.assertIn(
            "recipient_addresses(bcc recipients of candidate_message) is expected_bcc",
            exact_inventory,
        )
        self.assertIn("return id of item 1 of exact_drafts as string", script)
        self.assertIn("repeat with candidate_message in (matching_drafts)", script)
        self.assertLess(
            script.index("if (count of matching_sent)"),
            script.index("return id of item 1 of exact_drafts"),
        )
        self.assertNotIn("\n  send ", script.lower())

    def test_mail_settle_requires_two_consecutive_exact_observations(self):
        observations = [
            {},
            {"draftId": "draft-42"},
            {},
            {"draftId": "draft-42"},
            {"draftId": "draft-42"},
        ]
        with patch.object(
            mcp, "inspect_exact_mail_draft", side_effect=observations
        ) as inspect, patch.object(mcp.time, "sleep") as sleep:
            settled = mcp.wait_for_exact_mail_draft(
                [], [], [], "Subject", "Body", expected_draft_id="draft-42"
            )

        self.assertEqual(settled["draftId"], "draft-42")
        self.assertTrue(settled["settled"])
        self.assertEqual(inspect.call_count, 5)
        self.assertEqual(sleep.call_count, 4)

    def test_mail_settle_stops_immediately_on_verified_duplicates(self):
        duplicate = {
            "error": "Mail contains multiple identical unsent drafts.",
            "residual": True,
            "duplicates": True,
        }
        with patch.object(
            mcp, "inspect_exact_mail_draft", return_value=duplicate
        ) as inspect, patch.object(mcp.time, "sleep") as sleep:
            settled = mcp.wait_for_exact_mail_draft(
                [], [], [], "Subject", "Body", expected_draft_id="draft-42"
            )

        self.assertEqual(settled, duplicate)
        self.assertEqual(inspect.call_count, 1)
        sleep.assert_not_called()

    def test_mail_settle_stops_immediately_on_sent_or_content_collision(self):
        for collision in (
            {
                "error": "Mail contains a sent message with this subject and To recipient list.",
                "residual": True,
                "sent": True,
            },
            {
                "error": "Mail contains an unsent draft with differing content.",
                "residual": True,
                "collision": True,
            },
        ):
            with self.subTest(collision=collision), patch.object(
                mcp, "inspect_exact_mail_draft", return_value=collision
            ) as inspect, patch.object(mcp.time, "sleep") as sleep:
                settled = mcp.wait_for_exact_mail_draft(
                    [], [], [], "Subject", "Body", expected_draft_id="draft-42"
                )

            self.assertEqual(settled, collision)
            self.assertEqual(inspect.call_count, 1)
            sleep.assert_not_called()

    def test_mail_prepared_token_inventory_requires_a_durable_drafts_id(self):
        output = mcp.FIELD_SEPARATOR.join(
            ["OOMU_TOKEN_STATE", "saved-prepared-42", "1"]
        )
        with patch.object(mcp, "run_applescript", return_value=output) as run:
            inspected = mcp.inspect_mail_draft_by_subject_token("OOMU-token")

        self.assertEqual(inspected["draftId"], "saved-prepared-42")
        self.assertEqual(inspected["outgoingCount"], 1)
        self.assertTrue(inspected["present"])
        script = run.call_args.args[0]
        self.assertIn("every message of drafts mailbox", script)
        self.assertIn("every outgoing message", script)

    def test_mail_prepared_token_inventory_preserves_a_verified_absent_state(self):
        output = mcp.FIELD_SEPARATOR.join(["OOMU_TOKEN_STATE", "", "0"])
        with patch.object(mcp, "run_applescript", return_value=output):
            inspected = mcp.inspect_mail_draft_by_subject_token("OOMU-token")

        self.assertEqual(inspected["draftId"], "")
        self.assertEqual(inspected["outgoingCount"], 0)
        self.assertFalse(inspected["present"])

    def test_mail_operation_token_must_settle_absent_before_success(self):
        observations = [
            {"draftId": "saved-prepared-42", "outgoingCount": 1, "present": True},
            {"draftId": "", "outgoingCount": 0, "present": False},
            {"draftId": "", "outgoingCount": 0, "present": False},
        ]
        with patch.object(
            mcp, "inspect_mail_draft_by_subject_token", side_effect=observations
        ), patch.object(mcp.time, "sleep"):
            result = mcp.wait_for_mail_draft_subject_token_absence("OOMU-token")

        self.assertTrue(result["absent"])
        self.assertTrue(result["settled"])

    def test_mail_post_save_automation_error_can_recover_from_settled_exact_draft(self):
        with patch.object(
            mcp,
            "run_applescript",
            side_effect=[
                "ok",
                "ok",
                structured_error(
                    "execution_failed",
                    "Can’t make recipient reference into type string. (-1700)",
                ),
            ],
        ), clean_mail_baseline(), prepared_draft(), settled_draft(
            "saved-final-42"
        ), settled_token_absence(), patch.object(
            mcp, "remove_mail_draft_by_id"
        ) as cleanup:
            result = mcp.draft_system_email(
                {
                    "to": "owner@example.com",
                    "subject": "Subject",
                    "body": "First\n\nSecond",
                }
            )

        self.assertFalse(result["isError"])
        self.assertTrue(result["structuredContent"]["verified"])
        cleanup.assert_not_called()

    def test_mail_cleanup_verifies_saved_outgoing_and_sent_surfaces(self):
        absent = mcp.FIELD_SEPARATOR.join(["1", "1", "0", "0", "0", "0"])
        with patch.object(mcp, "run_applescript", return_value=absent) as run:
            cleanup = mcp.remove_mail_draft_by_id(
                "saved-draft-42",
                operation_token="OOMU-token",
                to_recipients=["owner@example.com"],
                cc_recipients=[],
                bcc_recipients=[],
                subject="Subject",
                body="Body",
            )

        self.assertEqual(cleanup["state"], "absent")
        script = run.call_args.args[0]
        self.assertIn("every message of drafts mailbox", script)
        self.assertIn("every outgoing message", script)
        self.assertIn("every message of sent mailbox", script)
        self.assertIn(
            "every message of drafts mailbox whose (subject is target_subject or subject is expected_subject)",
            script,
        )
        self.assertIn(
            "every outgoing message whose (subject is target_subject or subject is expected_subject)",
            script,
        )
        self.assertIn(
            "every message of sent mailbox whose (subject is target_subject or subject is expected_subject)",
            script,
        )
        self.assertIn('set target_id to "saved-draft-42"', script)
        self.assertIn('set target_subject to "OOMU-token"', script)
        self.assertNotIn("OOMU-CLEANUP-", script)
        self.assertNotIn("saving no", script)

    def test_mail_persisting_uncleared_cleanup_is_not_claimed_as_verified(self):
        persisted = mcp.FIELD_SEPARATOR.join(["1", "0", "1", "0", "0", "0"])
        with patch.object(mcp, "run_applescript", return_value=persisted):
            cleanup = mcp.remove_mail_draft_by_id("draft-42")

        self.assertEqual(cleanup["state"], "unverified")
        self.assertFalse(cleanup["verified"])
        self.assertTrue(cleanup["residual"])

    def test_mail_unverified_cleanup_emits_safe_typed_result(self):
        result = mcp.mail_draft_failure(
            "Mail could not verify the saved draft after creation.",
            mcp.mail_cleanup_result("unverified"),
            "cleanup",
        )

        self.assertEqual(
            result["structuredContent"]["code"], "mail_draft_result_unverified"
        )
        self.assertEqual(result["structuredContent"]["failurePhase"], "cleanup")
        self.assertFalse(result["structuredContent"]["cleanupVerified"])
        self.assertTrue(result["structuredContent"]["residualDraftPossible"])


class AppleAppResultPreservationTests(unittest.TestCase):
    def test_calendar_success_rows_survive_preflight_and_post_processing(self):
        row = mcp.FIELD_SEPARATOR.join(
            ["Work", "Product review", "9:00 AM", "10:00 AM", "Studio"]
        )
        with patch.object(mcp, "run_applescript", side_effect=["ok", row]):
            result = mcp.read_system_calendar(
                {
                    "start_date": "2026-07-13T09:00:00",
                    "end_date": "2026-07-13T10:00:00",
                }
            )

        self.assertFalse(result["isError"])
        self.assertEqual(result["structuredContent"]["events"][0]["name"], "Product review")

    def test_mail_success_rows_survive_preflight_and_post_processing(self):
        row = mcp.FIELD_SEPARATOR.join(
            ["sender@example.com", "Status", "Today", "false", "Ready to ship"]
        )
        with patch.object(mcp, "run_applescript", side_effect=["ok", row]):
            result = mcp.read_system_emails({"max_messages": 1, "unread_only": True})

        self.assertFalse(result["isError"])
        self.assertEqual(result["structuredContent"]["emails"][0]["subject"], "Status")

    def test_notes_success_rows_survive_post_processing(self):
        row = mcp.FIELD_SEPARATOR.join(
            ["Launch", "Yesterday", "Today", "Call Maya after the review"]
        )
        with patch.object(mcp, "run_applescript", return_value=row):
            result = mcp.read_system_notes({"max_notes": 1, "search_text": "Maya"})

        self.assertFalse(result["isError"])
        self.assertEqual(result["structuredContent"]["notes"][0]["title"], "Launch")

    def test_reminders_success_rows_survive_preflight_and_post_processing(self):
        row = mcp.FIELD_SEPARATOR.join(
            ["Reminders", "Call Maya", "Discuss launch", "Tomorrow"]
        )
        with patch.object(mcp, "run_applescript", side_effect=["ok", row]):
            result = mcp.read_system_reminders({"list_name": "Reminders"})

        self.assertFalse(result["isError"])
        self.assertEqual(result["structuredContent"]["reminders"][0]["title"], "Call Maya")

    def test_contacts_fallback_success_rows_survive_post_processing(self):
        row = mcp.FIELD_SEPARATOR.join(
            ["Maya", "Allan", "", "maya@example.com", "+1 555 0100"]
        )
        with patch.object(mcp, "run_applescript", return_value=row):
            result = mcp.read_system_contacts({"max_contacts": 1, "search_text": "Maya Allan"})

        self.assertFalse(result["isError"])
        self.assertEqual(result["structuredContent"]["contacts"][0]["name"], "Maya Allan")

    def test_messages_ui_success_rows_survive_post_processing(self):
        rows = mcp.RECORD_SEPARATOR.join(["Maya Allan", "Latest message"])
        with patch.object(mcp, "run_applescript", return_value=rows):
            result = mcp.read_apple_app_ui(
                {"app_name": "Messages", "activate": False, "max_items": 2}
            )

        self.assertFalse(result["isError"])
        self.assertEqual(result["structuredContent"]["uiText"], ["Maya Allan", "Latest message"])


if __name__ == "__main__":
    unittest.main()
