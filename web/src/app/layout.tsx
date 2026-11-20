import type { Metadata, Viewport } from "next";
import { Exo_2 } from "next/font/google";
import { AuthProvider } from "@/lib/auth";
import { ThemeProvider } from "@/lib/theme";
import { ToastProvider } from "@/lib/toast";
import "./globals.css";

const exo2 = Exo_2({
  subsets: ["latin"],
  variable: "--font-brand",
  weight: ["700"],
  display: "swap",
});

export const metadata: Metadata = {
  title: "Frona",
  description: "AI Agentic Assistant",
  manifest: "/manifest.webmanifest",
  appleWebApp: { capable: true, statusBarStyle: "black-translucent", title: "Frona" },
  icons: { apple: "/apple-touch-icon.png" },
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  maximumScale: 1,
  userScalable: false,
  viewportFit: "cover",
  themeColor: "#222831",
  // Shrink the layout when the soft keyboard opens instead of letting it cover
  // the page. The app is a full-height flex column (`h-[100dvh]`), so without
  // this the composer ends up hidden behind the keyboard on Android.
  interactiveWidget: "resizes-content",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className={`${exo2.variable} bg-surface text-text-primary antialiased`}>
        <ThemeProvider>
          <ToastProvider>
            <AuthProvider>{children}</AuthProvider>
          </ToastProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
