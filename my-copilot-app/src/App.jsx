import React from 'react';
import { CopilotKit } from "@copilotkit/react-core";
import { CopilotPopup } from "@copilotkit/react-ui";
import "@copilotkit/react-ui/styles.css";

const appStyles = `
  .container {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
  }
`;

// Этот стиль позволяет выделять и копировать текст из красного окна ошибки
const fixErrorSelectionStyles = `
  [data-copilotkit-error-popup] {
    user-select: text !important;
  }
`;

const App = () => {
  return (
    <>
      <style>{appStyles}</style>
      <style>{fixErrorSelectionStyles}</style>
      <CopilotKit
        runtimeUrl="http://127.0.0.1:8000/api/copilotkit"
        showDevConsole={true}
      >
        <div className="container">
          <h1>🎭 The Orchestrator</h1>
          <p>
            Нажмите на иконку в правом нижнем углу, чтобы начать чат.
          </p>
        </div>
        
        <CopilotPopup
          instructions="Отвечай всегда на русском языке."
          defaultOpen={true}
          labels={{
            title: "Чат с Оркестратором",
            initial: "Привет! Спроси меня что-нибудь.",
          }}
        />
      </CopilotKit>
    </>
  );
};

export default App;