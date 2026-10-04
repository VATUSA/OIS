import * as React from "react";
import {Button, Modal, Textarea, useToast} from "@ois/ui";

import {sendDiagnostics} from "@/lib/diagnostics";
import {can} from "@/lib/platform";

/**
 * "Send diagnostics…" (#629): asks for an optional note, then sends a report. Renders nothing off the
 * desktop — only the desktop app writes a log file, and only its session can send one.
 *
 * Sending is user-initiated only; nothing uploads in the background.
 */
export function SendDiagnosticsButton() {
  const toast = useToast();
  const [open, setOpen] = React.useState(false);
  const [note, setNote] = React.useState("");
  const [sending, setSending] = React.useState(false);

  if (!can("diagnostics")) return null;

  const send = async () => {
    setSending(true);
    try {
      await sendDiagnostics(note.trim());
      toast.success("Diagnostics sent", {description: "Thanks — staff can now see what happened."});
      setOpen(false);
      setNote("");
    } catch (error) {
      toast.error("Couldn't send diagnostics", {description: String(error)});
    } finally {
      setSending(false);
    }
  };

  return (
    <>
      <Button variant="secondary" onClick={() => setOpen(true)}>
        Send diagnostics…
      </Button>
      <Modal
        open={open}
        onClose={() => !sending && setOpen(false)}
        title="Send diagnostics"
        description="Sends this app's recent logs, its version, your OS and the page you're on to OIS staff, so they can investigate a problem. Tokens and sign-in codes are removed first. Reports are kept for 30 days."
        footer={
          <>
            <Button variant="secondary" onClick={() => setOpen(false)} disabled={sending}>
              Cancel
            </Button>
            <Button onClick={() => void send()} disabled={sending}>
              {sending ? "Sending…" : "Send"}
            </Button>
          </>
        }
      >
        <Textarea
          value={note}
          onChange={(e) => setNote(e.target.value)}
          maxLength={4000}
          rows={5}
          placeholder="What were you doing when it went wrong? (optional)"
          aria-label="What happened"
        />
      </Modal>
    </>
  );
}
