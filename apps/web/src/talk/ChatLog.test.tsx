import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { ChatLog, type ChatMessage } from "./ChatLog";

const MESSAGES: ChatMessage[] = [
  { id: "1", role: "tutor", text: "Hello! What is your name?" },
  { id: "2", role: "learner", text: "My name is Dewi.", edited: true },
];

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("ChatLog", () => {
  it("shows both sides in order and marks an edited message", async () => {
    renderApp(<ChatLog messages={MESSAGES} />);
    const log = await screen.findByRole("log", { name: "Conversation" });
    const items = within(log).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Hello! What is your name?");
    expect(items[1]).toHaveTextContent("My name is Dewi.");
    expect(items[1]).toHaveTextContent("(edited)");
  });

  it("renders learner text as text, never as HTML", async () => {
    const hostile: ChatMessage[] = [{ id: "x", role: "learner", text: '<img src=x onerror="alert(1)"><b>bold</b>' }];
    renderApp(<ChatLog messages={hostile} />);
    const log = await screen.findByRole("log");
    expect(log.querySelector("img")).toBeNull();
    expect(log.querySelector("b")).toBeNull();
    expect(log).toHaveTextContent('<img src=x onerror="alert(1)"><b>bold</b>');
  });

  it("is busy while a reply streams and announces the reply only once it is complete", async () => {
    const streaming: ChatMessage[] = [{ id: "1", role: "tutor", text: "Nice to meet", streaming: true }];
    const { rerender } = renderApp(<ChatLog messages={streaming} />);
    const log = await screen.findByRole("log");
    expect(log).toHaveAttribute("aria-busy", "true");
    expect(document.querySelector(".sr-only[aria-live]")).toHaveTextContent("");

    rerender(<ChatLog messages={[{ id: "1", role: "tutor", text: "Nice to meet you!" }]} />);
    expect(screen.getByRole("log")).not.toHaveAttribute("aria-busy");
    expect(document.querySelector(".sr-only[aria-live]")).toHaveTextContent("New message from Lumi: Nice to meet you!");
  });

  it("shows that Lumi is thinking before the reply starts", async () => {
    renderApp(<ChatLog messages={MESSAGES.slice(0, 1)} thinking />);
    expect(await screen.findByText("Lumi is thinking")).toBeInTheDocument();
    expect(screen.getByRole("log")).toHaveAttribute("aria-busy", "true");
  });
});
