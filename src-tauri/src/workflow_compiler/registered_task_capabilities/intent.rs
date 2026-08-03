pub(crate) fn has_send_intent(prompt: &str) -> bool {
    !prompt.contains("do not send")
        && !prompt.contains("never send")
        && (prompt.contains("send one email")
            || prompt.contains("send an email")
            || prompt.contains("send email"))
}
