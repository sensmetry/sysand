# Sysand Local Testing Makefile

# Variables
WORKSPACE := $(shell pwd)
SYSAND := cargo run --manifest-path $(WORKSPACE)/Cargo.toml -p sysand --
TEST_ROOT := /tmp/sysand-test
TEST1_DIR := $(TEST_ROOT)/test1
TEST2_DIR := $(TEST_ROOT)/test2
TEST_USE_ROOT := /tmp/sysand-test-use
TEST_USE_DIR := $(TEST_USE_ROOT)/consumer
INDEX_URL := https://sysand-index-alpha.syside.app
INDEX_READ_URL := https://sysand-index-alpha.syside.app/index/

# Default target
.PHONY: help
help:
	@echo "Sysand Testing Makefile"
	@echo ""
	@echo "Available targets:"
	@echo "  setup          - Initialize test projects (test1 and test2)"
	@echo "  publish        - Publish a project (requires auth, use PROJECT=test1 or test2)"
	@echo "  test-index-use - Test resolving dependencies from the index (public reads)"
	@echo "  info           - Show project information"
	@echo "  clean          - Remove all test directories"
	@echo ""
	@echo "Authentication (required for publishing):"
	@echo "  export SYSAND_CRED_ALPHA='$(INDEX_URL)/**'"
	@echo "  export SYSAND_CRED_ALPHA_BEARER_TOKEN='sysand_u_your_token_here'"
	@echo ""
	@echo "Typical workflow:"
	@echo "  1. make setup                    # Create test projects"
	@echo "  2. make publish PROJECT=test1    # Publish test1"
	@echo "  3. make publish PROJECT=test2    # Publish test2"
	@echo "  4. make test-index-use           # Test fetching from index"

# Setup: Create test projects
.PHONY: setup
setup: clean
	@echo "==> Setting up test projects in $(TEST_ROOT)"
	@mkdir -p $(TEST_ROOT)

	@echo ""
	@echo "==> Creating test1 project..."
	@mkdir -p $(TEST1_DIR)
	@cd $(TEST1_DIR) && $(SYSAND) init \
		--name test1 \
		--version 0.1.0

	@echo "package Test1Package {" > $(TEST1_DIR)/model.sysml
	@echo "    part def ExamplePart {" >> $(TEST1_DIR)/model.sysml
	@echo "        doc /* A simple example part from test1 */" >> $(TEST1_DIR)/model.sysml
	@echo "    }" >> $(TEST1_DIR)/model.sysml
	@echo "}" >> $(TEST1_DIR)/model.sysml

	@cd $(TEST1_DIR) && $(SYSAND) include model.sysml
	@cd $(TEST1_DIR) && $(SYSAND) build
	@echo "✓ test1 built successfully"

	@echo ""
	@echo "==> Creating test2 project..."
	@mkdir -p $(TEST2_DIR)
	@cd $(TEST2_DIR) && $(SYSAND) init \
		--name test2 \
		--version 0.1.0

	@cd $(TEST2_DIR) && $(SYSAND) add "pkg:sysand/test1" "0.1.0" --no-lock --no-sync

	@echo "package Test2Package {" > $(TEST2_DIR)/model.sysml
	@echo "    import Test1Package::*;" >> $(TEST2_DIR)/model.sysml
	@echo "    " >> $(TEST2_DIR)/model.sysml
	@echo "    part def MyPart {" >> $(TEST2_DIR)/model.sysml
	@echo "        part example : ExamplePart;" >> $(TEST2_DIR)/model.sysml
	@echo "        doc /* Uses ExamplePart from test1 */" >> $(TEST2_DIR)/model.sysml
	@echo "    }" >> $(TEST2_DIR)/model.sysml
	@echo "}" >> $(TEST2_DIR)/model.sysml

	@cd $(TEST2_DIR) && $(SYSAND) include model.sysml
	@cd $(TEST2_DIR) && $(SYSAND) build
	@echo "✓ test2 built successfully"

	@echo ""
	@echo "==> Setup complete!"
	@echo "    test1: $(TEST1_DIR)"
	@echo "    test2: $(TEST2_DIR)"
	@echo ""
	@echo "To publish, run:"
	@echo "  make publish PROJECT=test1"
	@echo "  make publish PROJECT=test2"

# Publish: Publish a project to the index
.PHONY: publish
publish:
	@if [ -z "$(PROJECT)" ]; then \
		echo "Error: PROJECT variable not set"; \
		echo "Usage: make publish PROJECT=test1"; \
		exit 1; \
	fi
	@if [ -z "$$SYSAND_CRED_ALPHA_BEARER_TOKEN" ]; then \
		echo "Error: Authentication not configured"; \
		echo ""; \
		echo "Please set the following environment variables:"; \
		echo "  export SYSAND_CRED_ALPHA='$(INDEX_URL)/**'"; \
		echo "  export SYSAND_CRED_ALPHA_BEARER_TOKEN='sysand_u_your_token_here'"; \
		echo ""; \
		echo "Then run: make publish PROJECT=$(PROJECT)"; \
		exit 1; \
	fi
	@if [ ! -d "$(TEST_ROOT)/$(PROJECT)" ]; then \
		echo "Error: Project $(PROJECT) not found in $(TEST_ROOT)"; \
		echo "Run 'make setup' first"; \
		exit 1; \
	fi
	@echo "==> Publishing $(PROJECT) to $(INDEX_URL)..."
	@cd $(TEST_ROOT)/$(PROJECT) && $(SYSAND) publish --default-index $(INDEX_URL)
	@echo "✓ $(PROJECT) published successfully"

# Test index usage: Create a consumer project that depends on test2 from the index
.PHONY: test-index-use
test-index-use:
	@echo "==> Setting up consumer project in $(TEST_USE_ROOT)"
	@rm -rf $(TEST_USE_ROOT)
	@mkdir -p $(TEST_USE_DIR)

	@echo ""
	@echo "==> Creating consumer project..."
	@cd $(TEST_USE_DIR) && $(SYSAND) init --name consumer --version 0.1.0

	@echo ""
	@echo "==> Adding dependency on pkg:sysand/test2 from index..."
	@cd $(TEST_USE_DIR) && $(SYSAND) add "pkg:sysand/test2" "^0.1.0" --default-index $(INDEX_READ_URL)

	@echo ""
	@echo "==> Creating model file that uses test2..."
	@echo "package ConsumerPackage {" > $(TEST_USE_DIR)/consumer.sysml
	@echo "    import Test2Package::*;" >> $(TEST_USE_DIR)/consumer.sysml
	@echo "    " >> $(TEST_USE_DIR)/consumer.sysml
	@echo "    part def ConsumerPart {" >> $(TEST_USE_DIR)/consumer.sysml
	@echo "        part myPart : MyPart;" >> $(TEST_USE_DIR)/consumer.sysml
	@echo "        doc /* Uses MyPart from test2, which uses ExamplePart from test1 */" >> $(TEST_USE_DIR)/consumer.sysml
	@echo "    }" >> $(TEST_USE_DIR)/consumer.sysml
	@echo "}" >> $(TEST_USE_DIR)/consumer.sysml

	@cd $(TEST_USE_DIR) && $(SYSAND) include consumer.sysml
	@cd $(TEST_USE_DIR) && $(SYSAND) build

	@echo ""
	@echo "==> Test index usage complete!"
	@echo "    Consumer project: $(TEST_USE_DIR)"
	@echo "    Dependencies resolved from: $(INDEX_READ_URL)"
	@echo ""
	@echo "To inspect:"
	@echo "  cd $(TEST_USE_DIR)"
	@echo "  $(SYSAND) info"

# Clean: Remove test directories
.PHONY: clean
clean:
	@echo "==> Cleaning test directories..."
	@rm -rf $(TEST_ROOT)
	@rm -rf $(TEST_USE_ROOT)
	@echo "✓ Cleaned $(TEST_ROOT) and $(TEST_USE_ROOT)"

# Info: Show project information
.PHONY: info
info:
	@echo "==> Test1 Project Info:"
	@if [ -d "$(TEST1_DIR)" ]; then \
		cd $(TEST1_DIR) && $(SYSAND) info; \
	else \
		echo "Not initialized. Run 'make setup' first."; \
	fi
	@echo ""
	@echo "==> Test2 Project Info:"
	@if [ -d "$(TEST2_DIR)" ]; then \
		cd $(TEST2_DIR) && $(SYSAND) info; \
	else \
		echo "Not initialized. Run 'make setup' first."; \
	fi
