// src/App.jsx - КОД С ИСПРАВЛЕНИЕМ КОПИРОВАНИЯ

import React from 'react';
import { CopilotKit } from "@copilotkit/react-core";
import { CopilotPopup } from "@copilotkit/react-ui";
import "@copilotkit/react-ui/styles.css";

const appStyles = `
  .container {
    display: flex; flex-direction: column; align-items: center;
    justify-content: center; height: 100vh;
  }
`;

// ЭТОТ CSS ПОЗВОЛЯЕТ КОПИРОВАТЬ ТЕКСТ ИЗ КРАСНОГО ОКНА ОШИБКИ
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
      <CopilotKit runtimeUrl="http://127.0.0.1:8000/api/copilotkit">
        <div className="container">
          <h1>Тест соединения</h1>
        </div>
        <CopilotPopup
          defaultOpen={true}
          labels={{
            title: "Тестовый чат",
            initial: "Это просто тест. Отправь любое сообщение. Если я отвечу - соединение работает.",
          }}
        />
      </CopilotKit>
    </>
  );
};

export default App;