export const TERMINAL_DOWNLOADS_LIST_PROMPT =
  "Go into terminal and run a directory listing of my Downloads directory using the ls command";

export const MAIL_READ_FAILURE_RESULT = {
  content: [
    {
      type: "text",
      text: JSON.stringify({
        warning: "execution_failed",
        error: "Mail could not read the inbox.",
        emails: [],
      }),
    },
  ],
  structuredContent: {
    maxMessages: 20,
    unreadOnly: true,
    warning: "execution_failed",
    error: "Mail could not read the inbox.",
    emails: [],
  },
  isError: true,
};
