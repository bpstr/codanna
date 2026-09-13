import { cookies } from "next/headers";

import { Mail } from "@/app/(app)/examples/mail/components/mail";
import { accounts, mails } from "@/app/(app)/examples/mail/data";

export default function MailPage() {
  const layout = cookies().get("react-resizable-panels:layout");

  const defaultLayout = layout ? JSON.parse(layout.value) : undefined;
  const defaultCollapsed = false;

  return (
    <>
      <div className="flex min-w-0 flex-col">
        <Mail
          accounts={accounts}
          mails={mails}
          defaultLayout={defaultLayout}
          defaultCollapsed={defaultCollapsed}
          navCollapsedSize={4}
        />
      </div>
    </>
  );
}
