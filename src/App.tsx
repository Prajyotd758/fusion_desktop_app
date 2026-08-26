import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import LanguageSelector from "./components/OptionSelector";
import "./App.css";

type TaskStatus =
  | "idle"
  | "recording"
  | "transcribing"
  | "thinking"
  | "scanning_image"
  | "executing"
  | "speaking"
  | "error";

const STATUS_LABELS: Record<TaskStatus, string> = {
  idle: "Idle",
  recording: "Listening...",
  transcribing: "Transcribing...",
  thinking: "Thinking...",
  scanning_image: "Scanning screen...",
  executing: "Running command...",
  speaking: "Responding...",
  error: "Something went wrong",
};

function App() {
  const [lastCommand, setLastCommand] = useState("");
  const [taskDone, setTaskDone] = useState(false); // true once execution finishes, gates the button
  const [taskStatus, setTaskStatus] = useState<TaskStatus>("idle");
  const [isOnline, setIsOnline] = useState(true);
  const [showSaveForm, setShowSaveForm] = useState(false);
  const [keywordInput, setKeywordInput] = useState("");
  const [saveError, setSaveError] = useState("");

  const prevStatusRef = useRef<TaskStatus>("idle");

  useEffect(() => {
    const unlistenStatus = listen<TaskStatus>("task-status", (event) => {
      const newStatus = event.payload;

      // task just finished (was busy, now idle) -> reveal the "add command" button
      if (newStatus === "idle" && prevStatusRef.current !== "idle") {
        setTaskDone(true);
      }

      // a fresh recording started -> hide the button from the previous run
      if (newStatus === "recording") {
        setTaskDone(false);
      }

      prevStatusRef.current = newStatus;
      setTaskStatus(newStatus);
    });

    const unlistenUserCommand = listen<string>("user-command", (event) => {
      setLastCommand(event.payload);
    });

    return () => {
      unlistenStatus.then((f) => f());
      unlistenUserCommand.then((f) => f());
    };
  }, []);

  useEffect(() => {
    invoke<boolean>("check_internet")
      .then(setIsOnline)
      .catch(() => setIsOnline(false));
  }, []);

  function handleAddToUserCommand() {
    setKeywordInput("");
    setSaveError("");
    setShowSaveForm(true);
  }

  async function handleSaveCommand() {
    try {
      await invoke("save_custom_command", { keyword: keywordInput });
      setShowSaveForm(false);
    } catch (err) {
      setSaveError(String(err));
    }
  }

  const isRecording = taskStatus === "recording";
  const isProcessing = !["idle", "recording", "error"].includes(taskStatus);
  const isError = taskStatus === "error";

  return (
    <div className="app">
      <div className="bg-glow" />

      {!isOnline && (
        <div className="offline-banner">
          <span className="offline-dot" />
          No internet connection
        </div>
      )}

      <div className="top-bar">
        <LanguageSelector />
      </div>

      <div className="shell">
        <div className="mic-stage">
          <div className={`mic-rings ${isRecording ? "active" : ""}`}>
            <span className="ring ring-1" />
            <span className="ring ring-2" />
            <span className="ring ring-3" />
            <div
              className={`mic-btn ${isRecording ? "recording" : ""} ${
                isProcessing ? "processing" : ""
              }`}
            >
              {isProcessing ? (
                <span className="spinner" />
              ) : (
                <svg width="26" height="26" viewBox="0 0 24 24" fill="none">
                  <path
                    d="M12 15a3 3 0 003-3V6a3 3 0 00-6 0v6a3 3 0 003 3z"
                    fill="currentColor"
                  />
                  <path
                    d="M19 11a7 7 0 01-14 0M12 18v3"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                  />
                </svg>
              )}
            </div>
          </div>

          <div
            className={`status-pill ${isRecording ? "recording" : ""} ${
              isError ? "error" : ""
            }`}
          >
            <span className="status-dot" />
            {STATUS_LABELS[taskStatus]}
          </div>
        </div>

        <div className={`transcript ${lastCommand ? "visible" : ""}`}>
          <span className="transcript-label">Last command</span>
          <p>{lastCommand || "Your command will appear here."}</p>

          <button
            className="add-command-btn"
            style={{
              visibility:
                taskDone && !showSaveForm && lastCommand ? "visible" : "hidden",
              opacity: taskDone && !showSaveForm && lastCommand ? 1 : 0,
              pointerEvents:
                taskDone && !showSaveForm && lastCommand ? "auto" : "none",
            }}
            onClick={handleAddToUserCommand}
          >
            + Add to user command
          </button>
        </div>

        {showSaveForm && (
          <div className="last-command">
            <span className="transcript-label">Save as command</span>
            <input
              className="keyword-input"
              type="text"
              placeholder="e.g. open workspace"
              value={keywordInput}
              onChange={(e) => setKeywordInput(e.target.value)}
              autoFocus
            />
            {saveError && <p className="save-error">{saveError}</p>}
            <div className="save-form-actions">
              <button className="add-command-btn" onClick={handleSaveCommand}>
                Save
              </button>
              <button
                className="cancel-btn"
                onClick={() => setShowSaveForm(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export default App;
