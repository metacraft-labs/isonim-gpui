## Automated tests for the Task Manager demo app using GpuiRenderer.
##
## Verifies ALL demo app interactions in headless mode:
## add, toggle, filter, remove, clear completed.
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r --path:../isonim/src \
##     --path:demos/task-manager/src \
##     --nimcache:nimcache/test_demo_app tests/test_demo_app.nim

import unittest
import std/strutils
import isonim/core/[signals, computation, owner]
import isonim_gpui/renderer
import isonim_gpui/bindings

# Import the demo app module
import main {.all.}

# ============================================================================
# Helpers
# ============================================================================

proc taskTexts(store: TaskStore): seq[string] =
  for t in store.filteredTasks.val:
    result.add t.text

proc allTaskTexts(store: TaskStore): seq[string] =
  for t in store.tasks.val:
    result.add t.text

proc taskDoneStates(store: TaskStore): seq[bool] =
  for t in store.tasks.val:
    result.add t.done

proc addTaskViaUI(app: TaskApp; text: string) =
  ## Simulate adding a task through the UI: set input value, click add.
  app.inputValue.val = text
  fireEvent(app.addBtn, "click")

# ============================================================================
# Test suites
# ============================================================================

suite "Demo App - Initial State":
  setup:
    gpui_reset_tree()
    resetCallbacks()
    resetIdCounter()

  test "initial_state_empty_task_list":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      check app.store.tasks.val.len == 0
      check app.store.filteredTasks.val.len == 0
      check app.store.activeCount.val == 0
      check app.store.completedCount.val == 0
      check app.store.filter.val == fAll

      # The section should show "No tasks" fallback
      check textContent(app.taskListSection).contains("No tasks")

      dispose()

  test "initial_state_no_footer":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      # Footer container should be empty when there are no tasks
      check childCount(app.footerContainer) == 0

      dispose()

suite "Demo App - Add Task":
  setup:
    gpui_reset_tree()
    resetCallbacks()
    resetIdCounter()

  test "add_single_task":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Buy groceries")

      check app.store.tasks.val.len == 1
      check app.store.tasks.val[0].text == "Buy groceries"
      check app.store.tasks.val[0].done == false
      check app.store.activeCount.val == 1

      dispose()

  test "add_multiple_tasks":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Task A")
      app.addTaskViaUI("Task B")
      app.addTaskViaUI("Task C")

      check app.store.tasks.val.len == 3
      check allTaskTexts(app.store) == @["Task A", "Task B", "Task C"]
      check app.store.activeCount.val == 3
      check app.store.completedCount.val == 0

      dispose()

  test "add_empty_task_ignored":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("")
      app.addTaskViaUI("   ")

      check app.store.tasks.val.len == 0

      dispose()

  test "add_task_clears_input":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Some task")

      check app.inputValue.val == ""

      dispose()

  test "add_task_shows_footer":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("First task")

      # Footer should now be visible
      check childCount(app.footerContainer) > 0

      dispose()

  test "add_task_updates_tree":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Sample task")

      # Section should no longer show "No tasks"
      let sectionText = textContent(app.taskListSection)
      check sectionText.contains("Sample task")
      check not sectionText.contains("No tasks")

      dispose()

suite "Demo App - Toggle Task":
  setup:
    gpui_reset_tree()
    resetCallbacks()
    resetIdCounter()

  test "toggle_task_completion":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("My task")
      check app.store.tasks.val[0].done == false

      let taskId = app.store.tasks.val[0].id
      app.store.toggleTask(taskId)

      check app.store.tasks.val[0].done == true
      check app.store.activeCount.val == 0
      check app.store.completedCount.val == 1

      dispose()

  test "toggle_task_twice_restores_state":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Toggle me")
      let taskId = app.store.tasks.val[0].id

      app.store.toggleTask(taskId)
      check app.store.tasks.val[0].done == true

      app.store.toggleTask(taskId)
      check app.store.tasks.val[0].done == false
      check app.store.activeCount.val == 1

      dispose()

  test "toggle_updates_counts":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("A")
      app.addTaskViaUI("B")
      app.addTaskViaUI("C")

      check app.store.activeCount.val == 3
      check app.store.completedCount.val == 0

      app.store.toggleTask(app.store.tasks.val[0].id)
      check app.store.activeCount.val == 2
      check app.store.completedCount.val == 1

      app.store.toggleTask(app.store.tasks.val[1].id)
      check app.store.activeCount.val == 1
      check app.store.completedCount.val == 2

      dispose()

suite "Demo App - Filter":
  setup:
    gpui_reset_tree()
    resetCallbacks()
    resetIdCounter()

  test "filter_active":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Active task")
      app.addTaskViaUI("Completed task")
      app.store.toggleTask(app.store.tasks.val[1].id)

      app.store.setFilter(fActive)

      check app.store.filteredTasks.val.len == 1
      check taskTexts(app.store) == @["Active task"]

      dispose()

  test "filter_completed":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Active task")
      app.addTaskViaUI("Completed task")
      app.store.toggleTask(app.store.tasks.val[1].id)

      app.store.setFilter(fCompleted)

      check app.store.filteredTasks.val.len == 1
      check taskTexts(app.store) == @["Completed task"]

      dispose()

  test "filter_all":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Active task")
      app.addTaskViaUI("Completed task")
      app.store.toggleTask(app.store.tasks.val[1].id)

      app.store.setFilter(fActive)
      check app.store.filteredTasks.val.len == 1

      app.store.setFilter(fAll)
      check app.store.filteredTasks.val.len == 2
      check taskTexts(app.store) == @["Active task", "Completed task"]

      dispose()

  test "filter_via_ui_buttons":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Task 1")
      app.addTaskViaUI("Task 2")
      app.store.toggleTask(app.store.tasks.val[0].id)

      fireEvent(app.filterBtnActive, "click")
      check app.store.filter.val == fActive
      check app.store.filteredTasks.val.len == 1

      fireEvent(app.filterBtnCompleted, "click")
      check app.store.filter.val == fCompleted
      check app.store.filteredTasks.val.len == 1

      fireEvent(app.filterBtnAll, "click")
      check app.store.filter.val == fAll
      check app.store.filteredTasks.val.len == 2

      dispose()

  test "filter_empty_result_shows_no_tasks":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Active only")

      app.store.setFilter(fCompleted)
      check app.store.filteredTasks.val.len == 0

      # The render effect should produce "No tasks" text
      check textContent(app.taskListSection).contains("No tasks")

      dispose()

suite "Demo App - Remove Task":
  setup:
    gpui_reset_tree()
    resetCallbacks()
    resetIdCounter()

  test "remove_task":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Keep me")
      app.addTaskViaUI("Remove me")

      let removeId = app.store.tasks.val[1].id
      app.store.removeTask(removeId)

      check app.store.tasks.val.len == 1
      check allTaskTexts(app.store) == @["Keep me"]

      dispose()

  test "remove_all_tasks_hides_footer":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Only task")
      check childCount(app.footerContainer) > 0

      app.store.removeTask(app.store.tasks.val[0].id)
      check app.store.tasks.val.len == 0

      # Footer should disappear
      check childCount(app.footerContainer) == 0

      dispose()

  test "remove_updates_counts":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("A")
      app.addTaskViaUI("B")
      app.addTaskViaUI("C")

      check app.store.activeCount.val == 3

      app.store.removeTask(app.store.tasks.val[1].id)
      check app.store.activeCount.val == 2
      check app.store.tasks.val.len == 2

      dispose()

suite "Demo App - Clear Completed":
  setup:
    gpui_reset_tree()
    resetCallbacks()
    resetIdCounter()

  test "clear_completed":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Active 1")
      app.addTaskViaUI("Done 1")
      app.addTaskViaUI("Active 2")
      app.addTaskViaUI("Done 2")

      app.store.toggleTask(app.store.tasks.val[1].id)
      app.store.toggleTask(app.store.tasks.val[3].id)

      check app.store.completedCount.val == 2
      check app.store.activeCount.val == 2

      app.store.clearCompleted()

      check app.store.tasks.val.len == 2
      check app.store.completedCount.val == 0
      check app.store.activeCount.val == 2
      check allTaskTexts(app.store) == @["Active 1", "Active 2"]

      dispose()

  test "clear_completed_via_ui_button":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Active")
      app.addTaskViaUI("To complete")
      app.store.toggleTask(app.store.tasks.val[1].id)

      fireEvent(app.clearCompletedBtn, "click")

      check app.store.tasks.val.len == 1
      check allTaskTexts(app.store) == @["Active"]

      dispose()

  test "clear_completed_with_none_completed":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("All active")
      app.store.clearCompleted()

      # Nothing should change
      check app.store.tasks.val.len == 1

      dispose()

suite "Demo App - Full User Flow":
  setup:
    gpui_reset_tree()
    resetCallbacks()
    resetIdCounter()

  test "full_user_flow":
    ## Comprehensive test exercising the full user workflow:
    ## add tasks -> toggle some -> filter -> clear completed -> verify
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      # 1. Start empty
      check app.store.tasks.val.len == 0
      check textContent(app.taskListSection).contains("No tasks")

      # 2. Add tasks
      app.addTaskViaUI("Learn Nim")
      app.addTaskViaUI("Build demo")
      app.addTaskViaUI("Write tests")
      app.addTaskViaUI("Deploy app")

      check app.store.tasks.val.len == 4
      check app.store.activeCount.val == 4
      check app.store.completedCount.val == 0

      # 3. Toggle some tasks as done
      app.store.toggleTask(app.store.tasks.val[0].id)  # "Learn Nim" -> done
      app.store.toggleTask(app.store.tasks.val[2].id)  # "Write tests" -> done

      check app.store.activeCount.val == 2
      check app.store.completedCount.val == 2
      check taskDoneStates(app.store) == @[true, false, true, false]

      # 4. Filter to active
      app.store.setFilter(fActive)
      check app.store.filteredTasks.val.len == 2
      check taskTexts(app.store) == @["Build demo", "Deploy app"]

      # 5. Filter to completed
      app.store.setFilter(fCompleted)
      check app.store.filteredTasks.val.len == 2
      check taskTexts(app.store) == @["Learn Nim", "Write tests"]

      # 6. Back to all
      app.store.setFilter(fAll)
      check app.store.filteredTasks.val.len == 4

      # 7. Remove one active task
      let deployId = app.store.tasks.val[3].id
      app.store.removeTask(deployId)
      check app.store.tasks.val.len == 3
      check app.store.activeCount.val == 1

      # 8. Clear completed
      app.store.clearCompleted()
      check app.store.tasks.val.len == 1
      check allTaskTexts(app.store) == @["Build demo"]
      check app.store.activeCount.val == 1
      check app.store.completedCount.val == 0

      # 9. Verify final UI state
      let sectionText = textContent(app.taskListSection)
      check not sectionText.contains("No tasks")
      check sectionText.contains("Build demo")

      dispose()

  test "reactive_count_label_updates":
    ## Verify the count label in the footer updates reactively.
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Task 1")
      app.addTaskViaUI("Task 2")
      app.addTaskViaUI("Task 3")

      # Count label should show "3 items left"
      check textContent(app.countLabel) == "3 items left"

      # Toggle one
      app.store.toggleTask(app.store.tasks.val[0].id)
      check textContent(app.countLabel) == "2 items left"

      # Toggle another
      app.store.toggleTask(app.store.tasks.val[1].id)
      check textContent(app.countLabel) == "1 item left"

      # Toggle all
      app.store.toggleTask(app.store.tasks.val[2].id)
      check textContent(app.countLabel) == "0 items left"

      dispose()

suite "Demo App - Tree Dump":
  setup:
    gpui_reset_tree()
    resetCallbacks()
    resetIdCounter()

  test "tree_dump_produces_output":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let app = buildTaskApp(r)

      app.addTaskViaUI("Sample task")

      let dump = dumpTree(app.root)
      check dump.len > 0
      # The root tree should contain the task text somewhere
      check textContent(app.root).contains("Sample task")

      dispose()
