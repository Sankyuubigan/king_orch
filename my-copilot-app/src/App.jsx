// src/App.jsx - ИСПРАВЛЕННАЯ ВЕРСИЯ
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
    text-align: center;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: white;
  }
  
  h1 { 
    color: white; 
    margin-bottom: 1rem;
    font-size: 2.5rem;
    text-shadow: 2px 2px 4px rgba(0,0,0,0.3);
  }
  
  p { 
    color: rgba(255,255,255,0.9); 
    font-size: 1.1rem;
    max-width: 500px;
    line-height: 1.6;
  }
  
  .status {
    margin-top: 2rem;
    padding: 1rem;
    background: rgba(255,255,255,0.1);
    border-radius: 10px;
    backdrop-filter: blur(10px);
  }
`;

const App = () => {
  return (
    <>
      <style>{appStyles}</style>
      <CopilotKit
        runtimeUrl="http://127.0.0.1:8000/copilotkit"
        showDevConsole={true}
      >
        <div className="container">
          <h1>🎭 The Orchestrator</h1>
          <p>
            Добро пожаловать в универсальный оркестратор! 
            Я могу искать в интернете, вычислять, получать системную информацию и многое другое.
          </p>
          <div className="status">
            <p>Нажмите на иконку чата в правом нижнем углу, чтобы начать диалог</p>
          </div>
        </div>
        
        <CopilotPopup
          instructions={`
            Ты - умный ассистент-оркестратор. Отвечай всегда на русском языке.
            
            У тебя есть доступ к следующим инструментам:
            - Поиск в интернете
            - Вычисления
            - Получение системной информации
            
            Всегда объясняй свои действия и показывай процесс мышления.
            Будь дружелюбным и полезным.
          `}
          defaultOpen={true}
          labels={{
            title: "Чат с Оркестратором 🎭",
            initial: "Привет! Я - Оркестратор. Спроси меня что-нибудь, и я воспользуюсь своими инструментами, чтобы помочь тебе!",
            placeholder: "Напиши свой вопрос...",
            send: "Отправить",
            stop: "Остановить"
          }}
        />
      </CopilotKit>
    </>
  );
};

export default App;