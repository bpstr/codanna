import Link from "next/link";

import { Announcement } from "@/components/announcement";
import { ExamplesNav } from "@/components/examples-nav";
import {
  PageActions,
  PageHeader,
  PageHeaderDescription,
  PageHeaderHeading,
} from "@/components/page-header";
import { Button } from "@/components/ui/button";
import MailPage from "@/app/(app)/examples/mail/page";
import { ThemeCustomizer } from "@/components/theming/ThemeCustomizer";
import { Card, CardContent, CardHeader } from "@/registry/default/ui/card";
import PageWrapper from "@/components/motion/PageWrapper";
import { ThemeSwitcher } from "@/components/theming/ThemeSwitcher";

export default function IndexPage() {
  return (
    <PageWrapper>
      <Card className="relative mb-4 md:fixed md:bottom-4 md:right-4 md:z-10">
        <CardHeader>Customize</CardHeader>
        <CardContent>
          <ThemeCustomizer />
        </CardContent>
      </Card>
      <div className="relative">
        <PageHeader>
          <Announcement />
          <PageHeaderHeading>Build your component library</PageHeaderHeading>
          <PageHeaderDescription>
            Beautifully designed components that you can copy and paste into
            your apps.
          </PageHeaderDescription>
          <PageActions>
            <Button asChild size="sm">
              <Link href="/docs">Get Started</Link>
            </Button>
            <Button asChild size="sm" variant="ghost">
              <Link target="_blank" rel="noreferrer" href="https://github.com">
                GitHub
              </Link>
            </Button>
          </PageActions>
        </PageHeader>
        <ExamplesNav className="[&>a:first-child]:text-primary" />
        <section className="min-w-0">
          <div className="overflow-hidden rounded-lg border bg-background shadow">
            <MailPage />
          </div>
        </section>
      </div>
    </PageWrapper>
  );
}
