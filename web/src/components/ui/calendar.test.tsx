import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { createElement, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DateLib } from "react-day-picker";
import { enUS, zhCN } from "react-day-picker/locale";
import { Calendar } from "./calendar";

type Scenario = "en-US" | "zh-CN" | "default" | "formatter" | "dateLib";

function renderCalendar(scenario: Scenario) {
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

function monthLabels(
  context: TestContext,
  scenario: Scenario,
  ambientLocale: "en-US" | "zh-CN",
) {
  // Control the unit's ambient formatting surface, not the OS/browser locale.
  // Explicit locales still use real Intl; TestContext restores this proxy.
  const nativeToLocaleString = Date.prototype.toLocaleString;
  context.mock.method(
    Date.prototype,
    "toLocaleString",
    function (
      this: Date,
      locales?: Intl.LocalesArgument,
      options?: Intl.DateTimeFormatOptions,
    ) {
      return nativeToLocaleString.call(
        this,
        locales === undefined || locales === "default"
          ? ambientLocale
          : locales,
        options,
      );
    },
  );
  const october = new Date(2026, 9, 1, 12);
  const ambientMonth = ambientLocale === "zh-CN" ? "10月" : "Oct";
  for (const locales of [undefined, "default"]) {
    assert.equal(
      october.toLocaleString(locales, { month: "short" }),
      ambientMonth,
    );
  }
  const explicitLocale = ambientLocale === "zh-CN" ? "en-US" : "zh-CN";
  const options: Intl.DateTimeFormatOptions = {
    month: "long",
    year: "numeric",
    timeZone: "Pacific/Kiritimati",
  };
  assert.equal(
    october.toLocaleString(explicitLocale, options),
    nativeToLocaleString.call(october, explicitLocale, options),
  );
  const dropdown =
    /<select\b[^>]*class="[^"]*\brdp-months_dropdown\b[^"]*"[^>]*>([\s\S]*?)<\/select>/.exec(
      renderCalendar(scenario),
    );
  assert(dropdown, "the real Calendar must render its month dropdown");
  return Array.from(
    dropdown[1].matchAll(/<option\b[^>]*value="\d+"[^>]*>([^<]*)<\/option>/g),
    (match) => match[1],
  );
}

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
test("month dropdown uses explicit English locale under controlled Chinese ambient", (context) => {
  assert.deepEqual(monthLabels(context, "en-US", "zh-CN"), englishMonths);
});
test("month dropdown uses explicit Chinese locale under controlled English ambient", (context) => {
  assert.deepEqual(
    monthLabels(context, "zh-CN", "en-US"),
    Array.from({ length: 12 }, (_, index) => `${index + 1}月`),
  );
});
test("month dropdown follows DayPicker's English default when locale is omitted", (context) => {
  assert.deepEqual(monthLabels(context, "default", "zh-CN"), englishMonths);
});
test("caller month formatter takes precedence", (context) => {
  assert.deepEqual(
    monthLabels(context, "formatter", "zh-CN"),
    Array.from({ length: 12 }, (_, index) => `Month ${index + 1}`),
  );
});
test("default month formatter uses the configured DateLib and time zone", (context) => {
  assert.deepEqual(
    monthLabels(context, "dateLib", "zh-CN"),
    Array.from(
      { length: 12 },
      (_, index) => `custom:${englishMonths[index]}:-840`,
    ),
  );
});
