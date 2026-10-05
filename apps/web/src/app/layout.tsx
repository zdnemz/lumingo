import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import { Providers } from "@/components/Providers";
import { BOOT_SCRIPT } from "@/state/preferences";
import "./globals.css";

export const metadata: Metadata = {
  title: "Lumingo",
  description: "An English tutor with a pixel-art game interface. It runs on your own computer.",
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    // The boot script sets these attributes before React hydrates, so React must not warn about them.
    // Without script the page stays still, in the night theme.
    <html lang="en" data-motion="off" data-theme="night" data-crt="off" suppressHydrationWarning>
      <head>
        {/* A constant string from our own source, never user data. */}
        {/* eslint-disable-next-line react/no-danger */}
        <script dangerouslySetInnerHTML={{ __html: BOOT_SCRIPT }} />
      </head>
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
