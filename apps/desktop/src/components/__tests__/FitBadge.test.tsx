import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { FitBadge, QualityStars } from "@/components/FitBadge";

describe("FitBadge", () => {
  it("renders the verdict label", () => {
    render(<FitBadge verdict="excellent" />);
    expect(screen.getByText("Excellent")).toBeInTheDocument();
  });

  it("renders the not-recommended label", () => {
    render(<FitBadge verdict="not_recommended" />);
    expect(screen.getByText("Not recommended")).toBeInTheDocument();
  });
});

describe("QualityStars", () => {
  it("exposes the tier via aria-label", () => {
    render(<QualityStars tier={3} />);
    expect(screen.getByLabelText("quality 3 of 5")).toBeInTheDocument();
  });
});
