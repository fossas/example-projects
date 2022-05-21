name := "scala-example-project"
version := "0.0.1"
scalaVersion := "3.1.0"
description := "Example sbt project that compiles using Scala and Sbt"

libraryDependencies ++= Seq(
  "net.sourceforge.htmlcleaner" % "htmlcleaner" % "2.26",
  "org.scalatest" % "scalatest_2.10" % "1.9.1" % "test"
)