import { describe, expect, it } from "vitest";
import { chatFailureNotice } from "./chatFailureNotice";

describe("chatFailureNotice", () => {
  it("localizes contextual file preparation without backend details", () => {
    const notice = chatFailureNotice({
      code: "contextual_file_preparation_failed",
      message: "The requested report folder could not be verified.",
    });

    expect(notice).toEqual({
      status: "Choose another folder",
      content:
        "OOMU couldn’t use that folder to create the file. Choose an existing folder you can access, then try again. Nothing was changed.",
    });
    expect(notice.content).not.toContain("contextual_file_preparation_failed");
    expect(notice.content).not.toContain("could not be verified");
  });

  it("explains a missing delete target without exposing the execution boundary", () => {
    const notice = chatFailureNotice({
      code: "delete_target_not_found",
      message: "Deletion boundary target resolution failed at /private/path.",
    });

    expect(notice).toEqual({
      status: "File not found",
      content: "That file is not there, so there is nothing to delete. Check the path and try again.",
    });
    expect(notice.content).not.toContain("boundary");
    expect(notice.content).not.toContain("/private/path");
  });

  it.each([
    "private_egress_destination_changed",
    "private_egress_payload_changed",
    "private_egress_signature_invalid",
    "private_egress_receipt_expired",
    "private_egress_receipt_consume_failed",
    "private_egress_receipt_unavailable",
    "private_egress_confirmation_unavailable",
    "private_egress_confirmation_expired",
    "private_egress_confirmation_invalid",
    "private_egress_signing_unavailable",
    "private_egress_receipt_store_failed",
    "private_source_invalid",
    "private_source_verification_unavailable",
    "private_provenance_invalid",
  ])("keeps private-egress failure %s plain and private", (code) => {
    const notice = chatFailureNotice({
      code,
      message: "internal receipt 123 at /private/customer-payroll.txt",
    });

    expect(notice).toEqual({
      status: "Private information stayed on this Mac",
      content:
        "OOMU couldn’t confirm permission to send your private information. Nothing was sent. Try again.",
    });
    expect(notice.content).not.toContain(code);
    expect(notice.content).not.toContain("/private/customer-payroll.txt");
  });
});
