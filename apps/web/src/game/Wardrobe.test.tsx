import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { Wardrobe, type CosmeticItem } from "./Wardrobe";

const ITEMS: CosmeticItem[] = [
  { id: "accessory-cap", kind: "accessory", rank: 1, unlocked: true, equipped: true },
  { id: "accessory-headphones", kind: "accessory", rank: 2, unlocked: true, equipped: false },
  { id: "theme-forest", kind: "theme", rank: 2, unlocked: true, equipped: false },
  { id: "theme-ember", kind: "theme", rank: 3, unlocked: false, equipped: false },
  { id: "accessory-crown", kind: "accessory", rank: 6, unlocked: false, equipped: false },
];

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("Wardrobe", () => {
  it("says rewards change looks and nothing else", () => {
    renderApp(<Wardrobe items={ITEMS} onEquip={vi.fn()} onUnequip={vi.fn()} />);
    expect(screen.getByText("Rewards for practising. They change how Lumingo looks and nothing else.")).toBeInTheDocument();
  });

  it("offers Wear for an unlocked item and takes the worn one off", async () => {
    const onEquip = vi.fn();
    const onUnequip = vi.fn();
    const user = userEvent.setup();
    renderApp(<Wardrobe items={ITEMS} onEquip={onEquip} onUnequip={onUnequip} />);
    await user.click(await screen.findByRole("button", { name: "Wear: Headphones" }));
    expect(onEquip).toHaveBeenCalledWith("accessory-headphones");
    await user.click(screen.getByRole("button", { name: "Take off: Red cap" }));
    expect(onUnequip).toHaveBeenCalledWith("accessory-cap");
  });

  it("offers Use for a theme", async () => {
    const onEquip = vi.fn();
    const user = userEvent.setup();
    renderApp(<Wardrobe items={ITEMS} onEquip={onEquip} onUnequip={vi.fn()} />);
    await user.click(await screen.findByRole("button", { name: "Use: Forest theme" }));
    expect(onEquip).toHaveBeenCalledWith("theme-forest");
  });

  it("shows a locked item as text with its rank and no button", async () => {
    renderApp(<Wardrobe items={ITEMS} onEquip={vi.fn()} onUnequip={vi.fn()} />);
    const grid = await screen.findByRole("list");
    const crown = within(grid).getByText("Crown").closest("li");
    expect(crown).toHaveTextContent("Unlocks at Glow rank 6");
    expect(within(crown as HTMLElement).queryByRole("button")).toBeNull();
  });
});
