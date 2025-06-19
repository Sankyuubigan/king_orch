# test_ollama.py
import requests
import json
import time
from langchain_community.llms import Ollama

def test_ollama_direct():
    """Test direct connection to Ollama API"""
    print("🔍 Testing direct Ollama API connection...")
    
    try:
        # Test basic connection
        response = requests.get("http://127.0.0.1:11434/api/tags", timeout=5)
        if response.status_code == 200:
            models = response.json()
            print(f"✅ Ollama is running. Available models: {[m['name'] for m in models.get('models', [])]}")
            return True
        else:
            print(f"❌ Ollama API returned status {response.status_code}")
            return False
    except Exception as e:
        print(f"❌ Failed to connect to Ollama: {e}")
        return False

def test_langchain_ollama():
    """Test Ollama through LangChain"""
    print("\n🔍 Testing Ollama through LangChain...")
    
    try:
        llm = Ollama(model="llama3:8b", base_url="http://127.0.0.1:11434")
        
        # Simple test
        response = llm.invoke("Say hello in one sentence")
        print(f"✅ LangChain Ollama response: {response}")
        return True
    except Exception as e:
        print(f"❌ LangChain Ollama test failed: {e}")
        return False

def test_agent_tools():
    """Test individual tools"""
    print("\n🔍 Testing agent tools...")
    
    try:
        from langchain_core.tools import tool
        
        @tool
        def get_system_info() -> str:
            """Get basic system information."""
            import platform
            return f"System: {platform.system()} {platform.release()}, Python: {platform.python_version()}"
        
        result = get_system_info.invoke({})
        print(f"✅ System info tool: {result}")
        
        @tool
        def calculate(expression: str) -> str:
            """Calculate mathematical expressions safely."""
            try:
                allowed_chars = set('0123456789+-*/.() ')
                if not all(c in allowed_chars for c in expression):
                    return "Invalid characters in mathematical expression"
                
                result = eval(expression)
                return f"Result: {result}"
            except Exception as e:
                return f"Calculation error: {str(e)}"
        
        calc_result = calculate.invoke({"expression": "2 + 2 * 3"})
        print(f"✅ Calculate tool: {calc_result}")
        
        return True
    except Exception as e:
        print(f"❌ Tools test failed: {e}")
        return False

def test_full_agent():
    """Test complete agent setup"""
    print("\n🔍 Testing complete agent setup...")
    
    try:
        from langchain_community.llms import Ollama
        from langchain.agents import AgentExecutor, create_react_agent
        from langchain_core.tools import tool
        from langchain.prompts import PromptTemplate
        
        # Initialize LLM
        llm = Ollama(model="llama3:8b", base_url="http://127.0.0.1:11434")
        
        # Create tools
        @tool
        def get_system_info() -> str:
            """Get basic system information."""
            import platform
            return f"System: {platform.system()} {platform.release()}, Python: {platform.python_version()}"

        @tool
        def calculate(expression: str) -> str:
            """Calculate mathematical expressions safely."""
            try:
                allowed_chars = set('0123456789+-*/.() ')
                if not all(c in allowed_chars for c in expression):
                    return "Invalid characters in mathematical expression"
                
                result = eval(expression)
                return f"Result: {result}"
            except Exception as e:
                return f"Calculation error: {str(e)}"
        
        tools = [get_system_info, calculate]
        
        # Create prompt
        prompt = PromptTemplate.from_template(
            """You are a helpful AI assistant. You have access to the following tools:

{tools}

Use the following format:

Question: the input question you must answer
Thought: you should always think about what to do
Action: the action to take, should be one of [{tool_names}]
Action Input: the input to the action
Observation: the result of the action
... (this Thought/Action/Action Input/Observation can repeat N times)
Thought: I now know the final answer
Final Answer: the final answer to the original input question

Begin!

Question: {input}
Thought: {agent_scratchpad}"""
        )
        
        # Create agent
        react_agent = create_react_agent(llm, tools, prompt)
        agent_executor = AgentExecutor(
            agent=react_agent, 
            tools=tools, 
            verbose=True,
            handle_parsing_errors=True,
            max_iterations=3
        )
        
        # Test the agent
        print("Testing agent with simple question...")
        result = agent_executor.invoke({"input": "What is 5 + 3?"})
        print(f"✅ Agent response: {result.get('output', 'No output')}")
        
        return True
    except Exception as e:
        print(f"❌ Full agent test failed: {e}")
        return False

if __name__ == "__main__":
    print("🚀 Starting Ollama and Agent tests...\n")
    
    # Wait a bit for Ollama to start if needed
    time.sleep(2)
    
    tests = [
        test_ollama_direct,
        test_langchain_ollama,
        test_agent_tools,
        test_full_agent
    ]
    
    results = []
    for test in tests:
        try:
            result = test()
            results.append(result)
        except Exception as e:
            print(f"❌ Test {test.__name__} crashed: {e}")
            results.append(False)
    
    print(f"\n📊 Test Results: {sum(results)}/{len(results)} passed")
    
    if all(results):
        print("🎉 All tests passed! Your setup should work correctly.")
    else:
        print("⚠️ Some tests failed. Check the errors above.")