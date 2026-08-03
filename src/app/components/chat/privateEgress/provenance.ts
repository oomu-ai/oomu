import type { ChatAttachment } from "../attachments";

type PrivateSourceKind =
  | "mail"
  | "calendar"
  | "contacts"
  | "photos"
  | "files"
  | "notes"
  | "reminders"
  | "messages"
  | "connector";

function sourceKind(name: string): PrivateSourceKind {
  const value = name.toLowerCase();
  if (value.includes("mail")) return "mail";
  if (value.includes("calendar")) return "calendar";
  if (value.includes("contact")) return "contacts";
  if (value.includes("photo")) return "photos";
  if (value.includes("note")) return "notes";
  if (value.includes("reminder")) return "reminders";
  if (value.includes("message")) return "messages";
  if (value.startsWith("connector_")) return "connector";
  return "files";
}

function sourceLabel(kind: PrivateSourceKind) {
  switch (kind) {
    case "mail": return "Mail on this Mac";
    case "calendar": return "Calendar on this Mac";
    case "contacts": return "Contacts on this Mac";
    case "photos": return "Photos on this Mac";
    case "notes": return "Notes on this Mac";
    case "reminders": return "Reminders on this Mac";
    case "messages": return "Messages on this Mac";
    case "connector": return "a connected private service";
    default: return "a local file";
  }
}

function exactBytes(attachment: ChatAttachment) {
  if (attachment.data_base64) {
    const binary = atob(attachment.data_base64);
    return Uint8Array.from(binary, (character) => character.charCodeAt(0));
  }
  return new TextEncoder().encode(attachment.text ?? "");
}

async function sha256(bytes: Uint8Array) {
  const digest = await crypto.subtle.digest("SHA-256", Uint8Array.from(bytes).buffer);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export async function attachPrivateDataProvenance(
  attachments: ChatAttachment[],
  localTurnId: string,
) {
  return Promise.all(attachments.map(async (attachment) => {
    if (
      attachment.name === "local_web_search.md" ||
      (!attachment.text && !attachment.data_base64)
    ) {
      return attachment;
    }
    const kind = sourceKind(attachment.name);
    return {
      ...attachment,
      private_data_provenance: {
        sourceKind: kind,
        sourceLabel: sourceLabel(kind),
        sourceDigest: await sha256(exactBytes(attachment)),
        sensitivity: "private",
        localTurnId,
        acquiredAtMs: Date.now(),
      },
    } satisfies ChatAttachment;
  }));
}
