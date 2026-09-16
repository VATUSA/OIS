// A plain-language, factual description of what OIS collects and why, derived from how the app
// actually works (VATSIM SSO identity, a session cookie, the boards/preferences a user creates, and
// action audit logs). It is descriptive, not a binding legal document — VATUSA should review and
// replace it with an official policy. Linked from the footer.

import {usePageHeader} from "@/components/shell/page-meta";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-2 border-t border-line-soft pt-6 first-of-type:border-t-0 first-of-type:pt-0">
      <h2 className="text-xl font-bold text-ink">{title}</h2>
      <div className="flex flex-col gap-2 text-[15px] leading-relaxed text-ink-2">{children}</div>
    </section>
  );
}

export function PrivacyPage() {
  usePageHeader({
    subtitle: "How the Operational Information System (OIS) handles your data. Last reviewed August 2026.",
  });

  return (
    <div className="flex w-full max-w-3xl flex-col gap-6">
      <div className="rounded-md border border-line bg-panel-2 p-4 text-sm leading-relaxed text-ink-2">
        This page describes OIS&apos;s current data practices in plain language. It is provided for
        transparency and is not a substitute for VATUSA&apos;s official policies or{" "}
        <a
          href="https://vatsim.net/"
          target="_blank"
          rel="noreferrer"
          className="text-brand-ink underline underline-offset-2 hover:text-ink"
        >
          VATSIM&apos;s Privacy Policy
        </a>
        , which govern your VATSIM account.
      </div>

      <Section title="Who operates OIS">
        <p>
          OIS is an operational tool run by VATUSA for use on the VATSIM network. It is not
          affiliated with any government or civil aviation authority, and it is not used for real-world
          flight operations.
        </p>
      </Section>

      <Section title="Information we collect">
        <p>You sign in with your VATSIM account through VATSIM Connect (single sign-on). We receive:</p>
        <ul className="ml-5 list-disc space-y-1 marker:text-ink-3">
          <li>Your VATSIM CID, name, controller rating, and email address.</li>
          <li>A session cookie that keeps you signed in.</li>
          <li>
            Content you create in the app — such as saved dashboards, layouts, and personal
            preferences.
          </li>
          <li>
            A record of actions that change data (for example creating a program or restriction),
            kept as an audit log for operational accountability.
          </li>
        </ul>
      </Section>

      <Section title="How we use it">
        <p>
          We use this information only to operate OIS: to sign you in, to determine what you are
          authorised to see and do, to save your work, and to keep an accurate operational record.
          We do not use your data for advertising, and we do not sell it.
        </p>
      </Section>

      <Section title="Cookies">
        <p>
          OIS sets a single cookie to store your sign-in session. It is required for the app to
          function and is not used for tracking or advertising. Signing out clears it.
        </p>
      </Section>

      <Section title="Who can see your information">
        <p>
          Your identity and activity are visible to VATUSA staff according to their role and area of
          responsibility, consistent with normal VATSIM division operations. We do not share your
          data with third parties outside the VATSIM ecosystem except where required to run the
          service.
        </p>
      </Section>

      <Section title="Retention">
        <p>
          Profile and content data are kept while your access is active. Operational audit records
          may be retained longer for accountability. Removal of your VATSIM membership is handled
          through VATSIM, and associated access to OIS is revoked accordingly.
        </p>
      </Section>

      <Section title="Your choices">
        <p>
          You can sign out at any time from the account menu. Your VATSIM account details are managed
          in your VATSIM profile. For questions about your data in OIS, contact VATUSA.
        </p>
      </Section>
    </div>
  );
}
