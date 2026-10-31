/** The action a failed job should offer, by stable error code. */
export type FailureAction = "retry" | "smaller_model" | "models";

export function failureAction(code: string | null | undefined): FailureAction {
  switch (code) {
    case "OUT_OF_MEMORY":
      return "smaller_model";
    case "MODEL_NOT_INSTALLED":
      return "models";
    default:
      return "retry";
  }
}
