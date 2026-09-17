//! Workflow routing: map a free-form request to the leadline workflow that
//! fits, with the exact command and a confidence gate.
//!
//! The catalog is fixed in code, so routing can only select a known workflow.
//! A low-confidence answer routes to `ask_user` instead of guessing, and
//! `not_leadline` exists so no-match requests have an honest answer.

import { TriageError, ask, choice } from "../client.ts";
import { choiceOf, confidenceOf, round } from "../compose.ts";
import type { ClientConfig, FeatureSpec, Questions, Usage } from "../types.ts";

export type Workflow = { description: string; command: string | null };

export const WORKFLOWS = {
  changed_review: {
    description:
      "Review what changed since a base revision and flag newly introduced complexity",
    command: "leadline changed --base <REV> --json",
  },
  gate_check: {
    description: "Gate a repository or path against complexity thresholds in CI",
    command: "leadline check <PATH> --cognitive 15 --cyclomatic 10 --max-nesting 4 --json",
  },
  function_explain: {
    description: "Explain one function's metrics and the lines that drive them",
    command: "leadline function <FILE> <NAME> --explain --json",
  },
  repo_overview: {
    description: "Summarize a repository: worst functions, risk, hotspots, and ownership",
    command: "leadline project <PATH> --since 90d --json",
  },
  dependency_impact: {
    description: "Find what depends on a file and how wide the blast radius is",
    command: "leadline impact <TARGET> --path <ROOT> --top 20",
  },
  hotspots: {
    description: "Rank files by complexity times churn",
    command: "leadline hotspots <PATH> --since 90d --json",
  },
  debt_review: {
    description:
      "Compare full debt and risk state against a base revision and gate regressions",
    command: "leadline debt --base <REV> --fail-on-regression --json",
  },
  security_triage: {
    description: "Triage scanner findings with code context",
    command: "leadline security --sarif <FILE> --json",
  },
  vulnerabilities_triage: {
    description: "Triage vulnerable dependencies with changed-import evidence",
    command: "leadline vulnerabilities --osv <FILE> --json",
  },
  duplication_triage: {
    description: "Find duplicated code and clone drift against a base revision",
    command: "leadline duplication <PATH> --base <REV> --json",
  },
  sql_review: {
    description: "Flag risky static SQL patterns",
    command: "leadline sql <PATH> --json",
  },
  sql_plan_review: {
    description: "Compare PostgreSQL plan regressions in checked-in EXPLAIN artifacts",
    command: "leadline sql-plan --current <DIR> --baseline <DIR>",
  },
  not_leadline: {
    description: "The request is not a leadline analysis task",
    command: null,
  },
} satisfies { [id: string]: Workflow };

/** Derived once from the catalog: the model sees descriptions as criteria, and
 * lookups do not rescan the table. */
const WORKFLOW_DESCRIPTIONS = Object.fromEntries(
  Object.entries(WORKFLOWS).map(([id, workflow]) => [id, workflow.description]),
);

const WORKFLOWS_BY_ID = Object.fromEntries(Object.entries(WORKFLOWS));

export const routeSpec: FeatureSpec = {
  id: "route",
  title: "Workflow routing",
  summary:
    "Routes a free-form request to the leadline workflow that fits, with the exact command and a confidence gate.",
  input: "text",
};

export type RouteState = { request: string; workflows: { [id: string]: string } };

export type RoutePlan = {
  feature: "route";
  request: string;
  batches: { state: RouteState; questions: Questions }[];
};

export type RouteAction = "run" | "ask_user" | "not_leadline";

/** Free-form request text is part of the outbound state, so it has a ceiling
 * like every other request dimension. */
export const MAX_ROUTE_CHARS = 8_000;

export type RouteAlternative = { workflow: string; probability: number };

export type RoutedRequest = {
  feature: "route";
  request: string;
  workflow: string | null;
  command: string | null;
  confidence: number | null;
  action: RouteAction;
  alternatives: RouteAlternative[];
  model: string | null;
  usage: Usage;
};

export function planRoute(options: { request: string }): RoutePlan {
  return {
    feature: "route",
    request: options.request,
    batches: [
      {
        state: routeState(options.request),
        questions: routeQuestions(),
      },
    ],
  };
}

export async function routeRequest(options: {
  request: string;
  config: ClientConfig;
  fetchImpl?: typeof fetch;
}): Promise<RoutedRequest> {
  if (options.request.length > MAX_ROUTE_CHARS) {
    throw new TriageError("route request exceeds 8000 characters");
  }

  const response = await ask({
    state: routeState(options.request),
    questions: routeQuestions(),
    config: options.config,
    fetchImpl: options.fetchImpl,
  });

  const answer = response.answers.workflow;

  if (answer === undefined || answer.type !== "choice") {
    throw new TriageError("TypeSafe did not return a workflow choice");
  }

  const workflow = choiceOf(answer);
  const confidence = confidenceOf(answer);

  const alternatives = Object.entries(answer.probabilities)
    .map(([id, probability]) => ({ workflow: id, probability: round(probability) }))
    .sort((left, right) => right.probability - left.probability)
    .slice(0, 3);

  const entry = workflow === null ? null : (WORKFLOWS_BY_ID[workflow] ?? null);

  return {
    feature: "route",
    request: options.request,
    workflow,
    command: entry === null ? null : entry.command,
    confidence,
    action: actionFor(workflow, entry, confidence),
    alternatives,
    model: response.model,
    usage: response.usage,
  };
}

/** Unknown workflows cannot be run: without a catalog entry there is no
 * command, so they route to the user instead of a null command. */
function actionFor(
  workflow: string | null,
  entry: Workflow | null,
  confidence: number | null,
): RouteAction {
  if (confidence !== null && confidence < 0.5) {
    return "ask_user";
  }

  if (workflow === "not_leadline") {
    return "not_leadline";
  }

  return entry === null ? "ask_user" : "run";
}

function routeState(request: string): RouteState {
  return { request, workflows: WORKFLOW_DESCRIPTIONS };
}

function routeQuestions(): Questions {
  return {
    workflow: choice(
      "Which leadline workflow best fits `request`? Choose the most specific workflow, or `not_leadline` when no analysis task fits.",
      WORKFLOW_DESCRIPTIONS,
    ),
  };
}
