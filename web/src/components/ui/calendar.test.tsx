import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DateLib } from "react-day-picker";
import { enUS, zhCN } from "react-day-picker/locale";
import { Calendar } from "./calendar";

type Scenario = "en-US" | "zh-CN" | "default" | "formatter" | "dateLib";

function renderCalendar(scenario: string) {
  const props: ComponentProps<typeof Calendar> = {
    month: new Date(2026, 9, 1, 12),
    today: new Date(2026, 9, 15, 12),
    startMonth: new Date(2026, 0, 1, 12),
    endMonth: new Date(2026, 11, 1, 12),
    captionLayout: "dropdown",
  };
  switch (scenario) {
    case "en-US":
      props.locale = enUS;
      break;
    case "zh-CN":
      props.locale = zhCN;
      break;
    case "default":
      break;
    case "formatter":
      props.locale = enUS;
      props.formatters = {
        formatMonthDropdown: (month) => `Month ${month.getMonth() + 1}`,
      };
      break;
    case "dateLib": {
      props.locale = enUS;
      props.timeZone = "Pacific/Kiritimati";
      const fallback = new DateLib({ locale: enUS, timeZone: props.timeZone });
      props.dateLib = {
        format: (date, token) => {
          if (token !== "LLL") return fallback.format(date, token);
          assert(date instanceof Date);
          return `custom:${fallback.format(date, token)}:${date.getTimezoneOffset()}`;
        },
      };
      break;
    }
    default:
      throw new Error(`Unknown calendar scenario: ${scenario}`);
  }
  return renderToStaticMarkup(createElement(Calendar, props));
}

function monthLabels(scenario: Scenario, environment: "en-US" | "zh-CN") {
  // Intl's default locale is fixed at process startup; use a real child locale.
  const result = spawnSync(
    process.execPath,
    [
      "--import",
      "tsx",
      fileURLToPath(import.meta.url),
      "--render-calendar",
      scenario,
    ],
    {
      cwd: fileURLToPath(new URL("../../..", import.meta.url)),
      env: {
        ...process.env,
        LANG: environment.replace("-", "_") + ".UTF-8",
        LC_ALL: environment.replace("-", "_") + ".UTF-8",
        TZ: "UTC",
      },
      encoding: "utf8",
    },
  );
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  const rendered: {
    html: string;
    environmentLocale: string;
    nativeMonth: string;
  } = JSON.parse(result.stdout);
  assert.equal(rendered.environmentLocale, environment);
  assert.equal(rendered.nativeMonth, environment === "zh-CN" ? "10月" : "Oct");
  const dropdown =
    /<select\b[^>]*class="[^"]*\brdp-months_dropdown\b[^"]*"[^>]*>([\s\S]*?)<\/select>/.exec(
      rendered.html,
    );
  assert(dropdown, "the real Calendar must render its month dropdown");
  return Array.from(
    dropdown[1].matchAll(/<option\b[^>]*value="\d+"[^>]*>([^<]*)<\/option>/g),
    (match) => match[1],
  );
}

if (process.argv[2] === "--render-calendar") {
  console.log(
    JSON.stringify({
      html: renderCalendar(process.argv[3]),
      environmentLocale: new Intl.DateTimeFormat().resolvedOptions().locale,
      nativeMonth: new Date(2026, 9, 1, 12).toLocaleString("default", {
        month: "short",
      }),
    }),
  );
} else {
  const englishMonths = [
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
  ];
  test("month dropdown uses explicit English locale in a Chinese environment", () => {
    assert.deepEqual(monthLabels("en-US", "zh-CN"), englishMonths);
  });
  test("month dropdown uses explicit Chinese locale in an English environment", () => {
    assert.deepEqual(
      monthLabels("zh-CN", "en-US"),
      Array.from({ length: 12 }, (_, index) => `${index + 1}月`),
    );
  });
  test("month dropdown follows DayPicker's English default when locale is omitted", () => {
    assert.deepEqual(monthLabels("default", "zh-CN"), englishMonths);
  });
  test("caller month formatter takes precedence", () => {
    assert.deepEqual(
      monthLabels("formatter", "zh-CN"),
      Array.from({ length: 12 }, (_, index) => `Month ${index + 1}`),
    );
  });
  test("default month formatter uses the configured DateLib and time zone", () => {
    assert.deepEqual(
      monthLabels("dateLib", "zh-CN"),
      Array.from(
        { length: 12 },
        (_, index) => `custom:${englishMonths[index]}:-840`,
      ),
    );
  });
}
