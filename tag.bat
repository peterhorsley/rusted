@echo off
if "%1"=="" goto help
git branch | find "* main" > NUL & IF ERRORLEVEL 1 (
    echo Only the main branch should be tagged.
    goto end
) else (
    git tag -a %1 -m "%1"
    choice /m "Repo has been tagged as %1. Push now?"
    if errorlevel 1 git push origin %1
    goto end
)
:help
echo Usage: tag [M.m.m]
echo.
:end