import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { Sidebar } from "@/components/Sidebar";

describe("Sidebar", () => {
  it("renders all primary screens", () => {
    render(<Sidebar active="home" onNavigate={() => {}} />);
    for (const label of ["Home", "Jobs", "Models", "Machine", "Settings"]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

  it("marks the active screen with aria-current", () => {
    render(<Sidebar active="models" onNavigate={() => {}} />);
    expect(screen.getByRole("button", { name: "Models" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("button", { name: "Home" })).not.toHaveAttribute(
      "aria-current",
    );
  });

  it("calls onNavigate when a screen is clicked", async () => {
    const onNavigate = vi.fn();
    render(<Sidebar active="home" onNavigate={onNavigate} />);
    await userEvent.click(screen.getByRole("button", { name: "Jobs" }));
    expect(onNavigate).toHaveBeenCalledWith("jobs");
  });
});
