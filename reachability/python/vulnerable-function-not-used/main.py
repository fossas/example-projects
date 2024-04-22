from flask import Flask
from opentelemetry.instrumentation.flask import FlaskInstrumentor

app = Flask(__name__)

FlaskInstrumentor().instrument_app(app) # CVE-2023-43810

@app.route("/", methods=["UNKNOWN"])
def hello():
    return "Hello!"

if __name__ == "__main__":
    app.run(debug=True)