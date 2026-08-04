import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import "./OptionSelector.css";

type Choice = "local" | "claude" | "open_ai" | "none";

const OPTIONS: { value: Choice; label: string }[] = [
  { value: "local", label: "Local" },
  { value: "claude", label: "Claude" },
  { value: "open_ai", label: "OpenAI" },
  { value: "none", label: "None" },
];

const DEFAULT_CHOICE: Choice = "local";

const LANGUAGE_LABELS: Record<string, string> = {
  en: "English",
  hi: "Hindi",
  mr: "Marathi",
};

export default function LlmSelector() {
  const [current, setCurrent] = useState<Choice>(DEFAULT_CHOICE);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  const [currentLang, setCurrentLang] = useState<string>("en");
  const [selectedLangs, setSelectedLangs] = useState<string[]>(["en", "hi", "mr"]);
  const [langOpen, setLangOpen] = useState(false);
  const langRootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    invoke<Choice>("get_llm_choice")
      .then(setCurrent)
      .catch(() => setCurrent(DEFAULT_CHOICE));

    invoke<string>("get_current_language")
      .then(setCurrentLang)
      .catch(() => setCurrentLang("en"));
  }, []);

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
      if (langRootRef.current && !langRootRef.current.contains(e.target as Node)) {
        setLangOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  async function handleSelect(choice: Choice) {
    setCurrent(choice); // optimistic update
    setOpen(false);
    try {
      await invoke("set_llm_choice", { choice });
    } catch (err) {
      console.error("Failed to set LLM choice:", err);
    }
  }

  async function handleLangSelect(langCode: string) {
    setCurrentLang(langCode); // optimistic update
    setLangOpen(false);
    try {
      await invoke("set_current_language", { langCode });
    } catch (err) {
      console.error("Failed to set language:", err);
    }
  }

  const currentLabel =
    OPTIONS.find((o) => o.value === current)?.label ?? "Local";

  const currentLangLabel = LANGUAGE_LABELS[currentLang] ?? currentLang;

  return (
    <div className="selector-row">
      <div className="llm-selector" ref={rootRef}>
        <button
          className={`llm-selector-trigger ${open ? "open" : ""}`}
          onClick={() => setOpen((o) => !o)}
        >
          <span className="llm-dot" data-choice={current} />
          <span className="llm-label">{currentLabel}</span>
          <svg
            className="llm-chevron"
            width="10"
            height="6"
            viewBox="0 0 10 6"
            fill="none"
          >
            <path
              d="M1 1L5 5L9 1"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>

        {open && (
          <div className="llm-selector-menu">
            {OPTIONS.map((opt) => (
              <button
                key={opt.value}
                className={`llm-selector-option ${
                  opt.value === current ? "selected" : ""
                }`}
                onClick={() => handleSelect(opt.value)}
              >
                <span className="llm-dot" data-choice={opt.value} />
                {opt.label}
                {opt.value === current && (
                  <svg
                    className="llm-check"
                    width="12"
                    height="10"
                    viewBox="0 0 12 10"
                    fill="none"
                  >
                    <path
                      d="M1 5L4.5 8.5L11 1"
                      stroke="currentColor"
                      strokeWidth="1.6"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                )}
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="llm-selector" ref={langRootRef}>
        <button
          className={`llm-selector-trigger ${langOpen ? "open" : ""}`}
          onClick={() => setLangOpen((o) => !o)}
        >
          <span className="llm-label">{currentLangLabel}</span>
          <svg
            className="llm-chevron"
            width="10"
            height="6"
            viewBox="0 0 10 6"
            fill="none"
          >
            <path
              d="M1 1L5 5L9 1"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>

        {langOpen && (
          <div className="llm-selector-menu">
            {selectedLangs.map((code) => (
              <button
                key={code}
                className={`llm-selector-option ${
                  code === currentLang ? "selected" : ""
                }`}
                onClick={() => handleLangSelect(code)}
              >
                {LANGUAGE_LABELS[code] ?? code}
                {code === currentLang && (
                  <svg
                    className="llm-check"
                    width="12"
                    height="10"
                    viewBox="0 0 12 10"
                    fill="none"
                  >
                    <path
                      d="M1 5L4.5 8.5L11 1"
                      stroke="currentColor"
                      strokeWidth="1.6"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                )}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}