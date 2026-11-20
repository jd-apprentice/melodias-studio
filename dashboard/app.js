// Read-only dashboard for the melodias history database. It polls one JSON
// summary and redraws; there is no build step and no external dependency.

const REFRESH_MS = 60000;
const REFRESH_LABEL =
  REFRESH_MS % 60000 === 0
    ? `${REFRESH_MS / 60000} min`
    : `${Math.round(REFRESH_MS / 1000)} s`;
const TABLE_ROWS = 50;
const SVG_NS = "http://www.w3.org/2000/svg";

let timer = null;
let busy = false;

function node(tag, className = "", text = null) {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text !== null) element.textContent = text;
  return element;
}

function svg(tag, attrs = {}, text = null) {
  const element = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attrs)) {
    element.setAttribute(key, value);
  }
  if (text !== null) element.textContent = text;
  return element;
}

function fmt(value) {
  return value === null || value === undefined || Number.isNaN(value)
    ? "-"
    : Number(value).toFixed(1);
}

function money(value) {
  return value ? `$${Number(value).toFixed(3)}` : "-";
}

function when(iso) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso || "-";
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function duration(seconds) {
  if (seconds === null || seconds === undefined) return "-";
  if (seconds < 90) return `${Math.round(seconds)}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 90) return `${minutes}m`;
  return `${(seconds / 3600).toFixed(1)}h`;
}

function percent(part, total) {
  return total ? `${Math.round((100 * part) / total)}%` : "0%";
}

function showError(message) {
  const banner = document.getElementById("banner");
  banner.textContent = message;
  banner.hidden = false;
}

function renderLegend() {
  const legend = document.getElementById("legend");
  for (const [className, label] of [
    ["tech", "technical"],
    ["edit", "editorial"],
    ["total", "total"],
  ]) {
    const item = node("span");
    item.append(node("i", `swatch ${className}`), document.createTextNode(label));
    legend.append(item);
  }
}

function renderCards(data) {
  const totals = data.totals;
  const acceptedHint = totals.by_limit
    ? `${percent(totals.accepted, totals.runs)} accepted · ${totals.by_limit} by limit`
    : `${percent(totals.accepted, totals.runs)} of the history`;
  const cards = [
    ["Runs", String(totals.runs), data.rubrics.join(" · ") || "no rubric yet"],
    ["Accepted", `${totals.accepted}/${totals.runs}`, acceptedHint],
    ["Mean total", fmt(totals.total), "target 80"],
    ["Mean technical", fmt(totals.technical), "target 85"],
    ["Mean editorial", fmt(totals.editorial), "target 70"],
    ["Judge spend", money(totals.cost_usd), "recorded verdicts"],
    ["Recorded", duration(totals.seconds), "pipeline time"],
  ];
  const container = document.getElementById("cards");
  container.replaceChildren();
  for (const [label, value, hint] of cards) {
    const card = node("div", "card");
    card.append(node("div", "label", label));
    card.append(node("div", "value", value));
    card.append(node("div", "hint", hint));
    container.append(card);
  }
}

function drawSeries(parent, values, className, x, y, runs) {
  let path = "";
  let pen = false;
  values.forEach((value, index) => {
    if (value === null || value === undefined) {
      pen = false;
      return;
    }
    path += `${pen ? "L" : "M"}${x(index).toFixed(1)},${y(value).toFixed(1)} `;
    pen = true;
  });
  if (path) parent.append(svg("path", { d: path, class: `line ${className}` }));
  values.forEach((value, index) => {
    if (value === null || value === undefined) return;
    const dot = svg("circle", {
      cx: x(index),
      cy: y(value),
      r: 3.5,
      class: `dot ${className}`,
    });
    const name = className === "tech" ? "technical" : className;
    dot.append(
      svg(
        "title",
        {},
        `#${runs[index].id} · ${name} ${fmt(value)} · ${when(runs[index].started_at)}`,
      ),
    );
    parent.append(dot);
  });
}

function renderScoreChart(data) {
  const chart = document.getElementById("score-chart");
  chart.replaceChildren();
  const runs = [...data.runs].sort((a, b) => a.id - b.id);
  const width = 900;
  const height = 320;
  const left = 46;
  const right = 18;
  const top = 18;
  const bottom = 38;
  const plotWidth = width - left - right;
  const plotHeight = height - top - bottom;
  const x = (index) =>
    runs.length <= 1 ? left + plotWidth / 2 : left + (index / (runs.length - 1)) * plotWidth;
  const y = (value) => top + plotHeight - (Math.max(0, Math.min(100, value)) / 100) * plotHeight;

  for (const value of [0, 25, 50, 75, 100]) {
    chart.append(
      svg("line", {
        x1: left,
        x2: left + plotWidth,
        y1: y(value),
        y2: y(value),
        class: "grid",
      }),
      svg(
        "text",
        { x: left - 8, y: y(value) + 4, class: "axis", "text-anchor": "end" },
        String(value),
      ),
    );
  }

  chart.append(
    svg("line", {
      x1: left,
      x2: left + plotWidth,
      y1: y(80),
      y2: y(80),
      class: "target",
    }),
    svg("text", { x: left + 6, y: y(80) - 6, class: "target-label" }, "target 80"),
  );

  drawSeries(chart, runs.map((run) => run.technical), "tech", x, y, runs);
  drawSeries(chart, runs.map((run) => run.editorial), "edit", x, y, runs);
  drawSeries(chart, runs.map((run) => run.total), "total", x, y, runs);

  const step = Math.max(1, Math.ceil(runs.length / 8));
  runs.forEach((run, index) => {
    if (index % step !== 0 && index !== runs.length - 1) return;
    chart.append(
      svg(
        "text",
        { x: x(index), y: height - 14, class: "axis", "text-anchor": "middle" },
        `#${run.id}`,
      ),
    );
  });
}

function aggregateFindings(findings) {
  const byCode = new Map();
  for (const finding of findings) {
    if (!finding.delta || finding.delta >= 0) continue;
    const item = byCode.get(finding.code) || {
      code: finding.code,
      lost: 0,
      hits: 0,
    };
    item.lost += finding.delta;
    item.hits += 1;
    byCode.set(finding.code, item);
  }
  return [...byCode.values()].sort((a, b) => a.lost - b.lost);
}

function renderFindingsChart(data) {
  const chart = document.getElementById("findings-chart");
  chart.replaceChildren();
  const rows = aggregateFindings(data.findings).slice(0, 12);
  if (!rows.length) {
    chart.setAttribute("viewBox", "0 0 900 60");
    chart.append(
      svg(
        "text",
        { x: 450, y: 36, class: "note", "text-anchor": "middle" },
        "No lost points in this window.",
      ),
    );
    return;
  }
  const height = 20 + rows.length * 30;
  chart.setAttribute("viewBox", `0 0 900 ${height}`);
  const labelWidth = 150;
  const left = labelWidth + 12;
  const valueWidth = 120;
  const width = 900 - left - valueWidth;
  const longest = Math.max(...rows.map((row) => Math.abs(row.lost))) || 1;
  const barHeight = 16;

  rows.forEach((row, index) => {
    const top = 20 + index * 30;
    const barWidth = Math.max(3, (Math.abs(row.lost) / longest) * width);
    chart.append(
      svg(
        "text",
        { x: labelWidth, y: top + barHeight - 3, class: "row-label", "text-anchor": "end" },
        row.code,
      ),
      svg("rect", {
        x: left,
        y: top,
        width: barWidth,
        height: barHeight,
        rx: 3,
        class: "bar",
      }),
      svg(
        "text",
        { x: left + barWidth + 8, y: top + barHeight - 3, class: "row-value" },
        `${row.lost.toFixed(1)} · ${row.hits}×`,
      ),
    );
  });
}

function scoreCell(value, target) {
  const cell = node("td", "num", fmt(value));
  if (value !== null && value !== undefined) {
    cell.classList.add(value >= target ? "hit" : "miss");
  }
  return cell;
}

function renderRunsTable(data) {
  const table = document.getElementById("runs");
  table.replaceChildren();
  const head = node("thead");
  const headRow = node("tr");
  const columns = [
    ["#", "num"],
    ["When", ""],
    ["Slug", ""],
    ["Tech", "num"],
    ["Edit", "num"],
    ["Total", "num"],
    ["Cost", "num"],
    ["Time", "num"],
    ["Status", ""],
  ];
  for (const [label, className] of columns) {
    headRow.append(node("th", className, label));
  }
  head.append(headRow);
  table.append(head);

  const body = node("tbody");
  for (const run of data.runs.slice(0, TABLE_ROWS)) {
    const row = node("tr");
    row.append(node("td", "num", `#${run.id}`));
    row.append(node("td", "mono", when(run.started_at)));
    const slug = node("td", "slug", run.slug);
    slug.title = run.vibe || run.slug;
    row.append(slug);
    row.append(scoreCell(run.technical, 85));
    row.append(scoreCell(run.editorial, 70));
    row.append(scoreCell(run.total, 80));
    row.append(node("td", "num", money(run.cost_usd)));
    row.append(node("td", "num", duration(run.seconds)));
    const outcome = run.outcome || (run.accepted ? "accepted" : "below_target");
    const label =
      outcome === "accepted"
        ? "accepted"
        : outcome === "accepted_by_limit"
          ? "by limit"
          : "below";
    const status = node("td");
    const badge = node(
      "span",
      `badge ${outcome === "accepted" ? "ok" : outcome === "accepted_by_limit" ? "limit" : "low"}`,
      label,
    );
    if (run.outcome_reason) badge.title = run.outcome_reason;
    status.append(badge);
    row.append(status);
    body.append(row);
  }
  table.append(body);

  const shown = Math.min(TABLE_ROWS, data.runs.length);
  document.getElementById("runs-hint").textContent =
    `${shown} shown · ${data.totals.runs} total`;
}

function renderEmpty(data) {
  document.getElementById("empty").hidden = false;
  document.getElementById("empty-detail").textContent = data.exists
    ? `The database at ${data.db} has no runs yet. Finish a run, or run make backfill.`
    : `No database at ${data.db} yet. Finish a run, or run make backfill to score the runs already in output/.`;
  for (const id of ["score-panel", "findings-panel", "runs-panel"]) {
    document.getElementById(id).hidden = true;
  }
}

function render(data) {
  document.getElementById("banner").hidden = true;
  document.getElementById("db").textContent = data.db;
  const stamp = new Date(data.generated_at);
  document.getElementById("stamp").textContent = Number.isNaN(stamp.getTime())
    ? ""
    : `updated ${stamp.toLocaleTimeString()}`;
  renderCards(data);
  if (!data.runs.length) {
    renderEmpty(data);
    document.getElementById("footer").textContent =
      `polling every ${REFRESH_LABEL} · read-only · database ${data.db}`;
    return;
  }
  for (const id of ["score-panel", "findings-panel", "runs-panel"]) {
    document.getElementById(id).hidden = false;
  }
  document.getElementById("empty").hidden = true;
  renderScoreChart(data);
  renderFindingsChart(data);
  renderRunsTable(data);
  document.getElementById("footer").textContent =
    `polling every ${REFRESH_LABEL} · read-only · window ${data.limit} runs · rubric ` +
    (data.rubrics.join(", ") || "-");
}

function schedule() {
  clearTimeout(timer);
  timer = setTimeout(refresh, REFRESH_MS);
}

async function refresh() {
  if (busy || document.hidden) {
    schedule();
    return;
  }
  busy = true;
  try {
    const response = await fetch("/api/summary", { cache: "no-store" });
    const payload = await response.json().catch(() => null);
    if (!response.ok) {
      throw new Error((payload && payload.error) || `HTTP ${response.status}`);
    }
    render(payload);
    document.getElementById("live").classList.remove("down");
  } catch (error) {
    document.getElementById("live").classList.add("down");
    showError(error instanceof Error ? error.message : String(error));
  } finally {
    busy = false;
    schedule();
  }
}

document.addEventListener("visibilitychange", () => {
  if (!document.hidden) refresh();
});

renderLegend();
refresh();
