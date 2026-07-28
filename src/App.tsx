import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import "./App.css";

function App() {
  const [text, setText] = useState("");
  const [isRecording, setIsRecording] = useState(false);
  const [status, setStatus] = useState("Idle");
  const [isError, setIsError] = useState(false);

  useEffect(() => {
    const unlisten = listen<string>("command-result", (event) => {
      setText(event.payload);
      setStatus("Idle");
    });

    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function handleClick() {
    if (!isRecording) {
      try {
        setIsError(false);
        setStatus("Starting...");
        await invoke<string>("start_recording");
        setIsRecording(true);
        setStatus("Listening...");
      } catch (err) {
        setIsError(true);
        setStatus(`Error: ${err}`);
      }
    } else {
      try {
        setStatus("Transcribing...");
        setIsRecording(false);
        const result = await invoke<string>("stop_recording_and_transcribe");
        setText(result);
        setStatus("Idle");
      } catch (err) {
        setIsError(true);
        setStatus(`Error: ${err}`);
      }
    }
  }

  return (
    <div className="app">
      <div className="bg-glow" />

      <div className="shell">
        <header className="header">
          <span className="logo-dot" />
          <h1>Arceus Assistant</h1>

          <h2>to do list in this app</h2>
          <ul>
            <li>
              Add user commands : user will add commands that will perform a
              particular task <br />
              Eg. open workspace : will open vs code, browser, file explorer to
              project path then open cmd in that folder
            </li>
            <li>add login/registration</li>
            <li>executing command via api from online llm like gpt,claude</li>
            <li>connecting to users openai/anthropic account</li>
          </ul>
        </header>

        <div className={`transcript ${text ? "visible" : ""}`}>
          <span className="transcript-label">Last response</span>
          <p>{text || "Your command output will appear here."}</p>
        </div>
      </div>
    </div>
  );
}

export default App;
