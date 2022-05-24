name := "sbt-project-example"
organization in ThisBuild := "com.fossa"
scalaVersion in ThisBuild := "3.1.0"

lazy val global = project
  .in(file("."))
  .aggregate(
    sub1,
    sub2
  )

lazy val sub1 = project
  .settings(
    name := "sub1",
    libraryDependencies ++= Seq( 
        "com.typesafe.akka" %% "akka-stream" % "2.6.19",
        "net.logstash.logback" % "logstash-logback-encoder" % "7.2",
    )
  )

lazy val sub2 = project
  .settings(
    name := "sub2",
    libraryDependencies ++= Seq(
        "net.sourceforge.htmlcleaner" % "htmlcleaner" % "2.4", 
        "ch.qos.logback" % "logback-classic" % "1.2.11",
    )
  )